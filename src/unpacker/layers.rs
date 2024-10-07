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

use crate::{fs::Directory, EventHandler, UnpackError};

use super::{
    extractor::extract,
    manifests::{Blob, Manifest},
    try_io,
};

/// Maximum number of threads to download blobs in parallel.
const QUEUE_LIMIT: usize = 8;

const CONFIG_FILE: &str = "config.json";

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

    let download_tasks: Vec<_> = [(&manifest.config, Some(Path::new(CONFIG_FILE)))]
        .into_iter()
        .chain(manifest.layers.iter().map(|l| (l, None)))
        .map(|(blob, filename)| Download::new(blob, filename))
        .collect();

    // Download blobs in a thread pool.
    let pending: VecDeque<_> = download_tasks.iter().collect();
    let pending = Mutex::new(pending);

    thread::scope(|scope| {
        let alive_tracker = AliveTracker(&is_alive);

        // Launch a thread pool to download the blobs.
        for _ in 0..min(QUEUE_LIMIT, download_tasks.len()) {
            scope.spawn(|| {
                while let Ok(Some(task)) = pending.lock().map(|mut q| q.pop_front()) {
                    task.set(run_download(
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
        for task in &download_tasks {
            try_io!(
                task.blob.digest.hash(),
                extract(event_handler, &target, task.blob, task.get()?),
            )
        }

        drop(alive_tracker);

        event_handler.finished();

        Ok(())
    })
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

    fn set(&self, result: Result<File, UnpackError>) {
        let mut lock = self.result.lock().unwrap();
        *lock = Some(result);
        self.notifier.notify_one();
    }

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
        digest.hash(),
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

        let n = try_io!(digest.hash(), input.read(&mut data[..]));

        if n == 0 {
            drop(output);
            return Ok(file);
        }

        event_handler.download_progress_bytes(n);

        try_io!(digest.hash(), output.write_all(&data[..n]));
    }
}
