use std::{
    cmp::min,
    collections::VecDeque,
    fs::File,
    io::{self, BufWriter, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    thread,
};

use rustix::fs::Mode;

use crate::{
    fs::{normalize_path, DirFdCache, Directory},
    manifests::{Blob, Manifest},
    EventHandler,
};

use super::{layers::unpack_layer, try_io, UnpackError};

/// Maximum number of threads to download blobs in parallel.
const QUEUE_LIMIT: usize = 8;

/// File to store the configuration.
const CONFIG_PATH: &str = "config.json";

/// Directory to store layers.
const ROOTFS_PATH: &str = "rootfs";

pub(crate) fn get<E: EventHandler>(
    http_client: crate::http::Client<E>,
    manifest: Manifest,
    target: &Path,
    event_handler: &E,
) -> Result<(), UnpackError> {
    let is_alive = AtomicBool::new(true);

    let target = try_io!(target, Directory::new(target));

    event_handler.download_start(
        manifest.layers.len(),
        manifest.config.size + manifest.layers.iter().fold(0, |a, l| a + l.size),
    );

    let download_tasks: Vec<_> = [(&manifest.config, Some(Path::new(CONFIG_PATH)))]
        .into_iter()
        .chain(manifest.layers.iter().map(|l| (l, None)))
        .map(|(blob, filename)| Download::new(blob, filename))
        .collect();

    // Download blobs in a thread pool.
    let pending: VecDeque<_> = download_tasks.iter().collect();
    let pending = Mutex::new(pending);

    // Disable umask.
    let _umask_guard = UmaskGuard(rustix::process::umask(Mode::empty()));

    thread::scope(|scope| {
        let alive_tracker = AliveTracker(&is_alive);

        // Launch a thread pool to download the blobs.
        for _ in 0..min(QUEUE_LIMIT, download_tasks.len()) {
            scope.spawn(|| {
                while let Ok(Some(task)) = pending.lock().map(|mut q| q.pop_front()) {
                    task.complete(run_download(
                        &target,
                        task,
                        &http_client,
                        event_handler,
                        &is_alive,
                    ));
                }
            });
        }

        // Get downloaded files and extract them.

        let rootfs = Directory::from(try_io!(
            ROOTFS_PATH,
            target.open_directory(ROOTFS_PATH, true)
        ));

        let mut dirs_mtimes = Default::default();

        for task in &download_tasks {
            unpack_layer(
                task.blob.digest.source(),
                event_handler,
                &rootfs,
                task.blob,
                task.get()?,
                &mut dirs_mtimes,
            )?;
        }

        drop(alive_tracker);

        // Update the mtime of the directories after all files are extracted.
        //
        // This can't be done before because extracting new files updates
        // the mtime of the parent directory.

        let mut dirs_cache = DirFdCache::new(&rootfs);
        for ((_, path), entry) in dirs_mtimes {
            let mut update = || -> io::Result<()> {
                use rustix::fs;

                let (parent_path, file_name) = normalize_path(&path)?;

                let parent = dirs_cache.get(&parent_path, false)?;

                let mtime = fs::Timespec {
                    tv_sec: entry.mtime as i64,
                    tv_nsec: 0,
                };

                let times = fs::Timestamps {
                    last_access: mtime,
                    last_modification: mtime,
                };

                crate::fs::change_owner(parent, &file_name, entry.uid, entry.gid, false)?;
                fs::chmodat(parent, &file_name, entry.mode, fs::AtFlags::empty())?;
                fs::utimensat(parent, &file_name, &times, fs::AtFlags::SYMLINK_NOFOLLOW)?;

                Ok(())
            };

            // Ignore NotFound errors. Those may happen because whiteout entries
            // removed directories created by lower layers.
            if let Err(e) = update() {
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(UnpackError::Io(e, path));
                }
            }
        }

        event_handler.finished();

        Ok(())
    })
}

/// Store the previous value for umask, to restore it on drop.
struct UmaskGuard(Mode);

impl Drop for UmaskGuard {
    fn drop(&mut self) {
        rustix::process::umask(self.0);
    }
}

/// Set the `AtomicBool` instance to `false` when this instance is
/// dropped (for example, after `panic!`).
struct AliveTracker<'a>(&'a AtomicBool);

impl Drop for AliveTracker<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

struct Download<'a> {
    blob: &'a Blob,
    filename: Option<&'a Path>,
    result: Mutex<Option<Result<File, UnpackError>>>,
    notifier: Condvar,
}

impl<'a> Download<'a> {
    fn new(blob: &'a Blob, filename: Option<&'a Path>) -> Self {
        Self {
            blob,
            filename,
            result: Default::default(),
            notifier: Condvar::new(),
        }
    }

    /// Store the result of a download operation, and notify
    /// any waiting thread.
    fn complete(&self, result: Result<File, UnpackError>) {
        let mut lock = self.result.lock().unwrap();
        *lock = Some(result);
        self.notifier.notify_one();
    }

    /// Wait until the result of a download is ready.
    fn get(&self) -> Result<File, UnpackError> {
        let mut lock = self.result.lock().unwrap();
        loop {
            lock = match lock.take() {
                Some(r) => return r,
                None => self.notifier.wait(lock).unwrap(),
            }
        }
    }
}

/// Download a blob from the HTTP server. Return the file where
/// its contents are written.
fn run_download<E: EventHandler>(
    target: &Directory,
    task: &Download,
    http_client: &crate::http::Client<E>,
    event_handler: &impl EventHandler,
    is_alive: &AtomicBool,
) -> Result<File, UnpackError> {
    let digest = &task.blob.digest;

    let mut input = http_client.download_blob(digest)?;

    let fd = try_io!(
        digest.source(),
        match task.filename {
            Some(n) => target.create(n, Mode::RUSR | Mode::WUSR),
            None => target.tmpfile(),
        },
    );

    let mut file = File::from(fd);

    let mut data = [0u8; 8 * 1024];
    let mut output = BufWriter::new(&mut file);

    loop {
        if !is_alive.load(Ordering::Relaxed) {
            return Err(UnpackError::Interrupted);
        }

        let n = try_io!(digest.source(), input.read(&mut data[..]));

        if n == 0 {
            drop(output);
            return Ok(file);
        }

        event_handler.download_progress_bytes(n);

        try_io!(digest.source(), output.write_all(&data[..n]));
    }
}
