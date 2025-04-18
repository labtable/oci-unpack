use std::io::{
    self, BufReader,
    ErrorKind::{AlreadyExists, NotFound},
    Read, Seek,
};

use std::{
    cell::Cell,
    ffi::OsStr,
    fs::File,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use rustix::{
    fd::{AsFd, BorrowedFd, OwnedFd},
    fs::Mode,
};

use crate::{
    fs::{normalize_path, DirFdCache, Directory},
    manifests::Blob,
    EventHandler, MediaType,
};

use super::{try_io, DirectoryMetadata, UnpackError};

const WHITEOUT_PREFIX: &[u8] = b".wh.";

const WHITEOUT_OPAQUE: &[u8] = b".wh..opq";

pub(crate) fn unpack_layer<E: EventHandler>(
    blob_id: &str,
    event_handler: &E,
    target: &Directory,
    blob: &Blob,
    mut tarball: File,
    dirs_metadata: &mut DirectoryMetadata,
) -> Result<(), UnpackError> {
    let archive_len = try_io!(blob_id, {
        let len = tarball.seek(io::SeekFrom::End(0))?;
        tarball.rewind()?;
        len
    });

    // Track position (in bytes) to send progress notifications.
    let tarball_position = Cell::new(0);
    let tarball = PositionTracker {
        count: &tarball_position,
        reader: tarball,
    };

    // Uncompress and extract files from the archive.
    let reader: Box<dyn Read> = match blob.media_type {
        MediaType::DockerImageV1 | MediaType::OciConfig => {
            // Configuration files are just written to disk.
            return Ok(());
        }

        MediaType::DockerFsTarGzip | MediaType::OciFsTarGzip => {
            Box::new(flate2::read::GzDecoder::new(tarball))
        }

        #[cfg(feature = "zstd")]
        MediaType::OciFsTarZstd => {
            let reader = zstd::stream::read::Decoder::new(tarball)
                .map_err(|e| UnpackError::Io(e, format!("blob:{blob_id}").into()))?;

            Box::new(reader)
        }

        MediaType::OciFsTar => Box::new(BufReader::new(tarball)),

        unknown => return Err(UnpackError::InvalidContentType(unknown)),
    };

    event_handler.layer_start(archive_len);

    let mut archive = tar::Archive::new(reader);
    let mut ctx = Context::new(event_handler, blob_id, target, dirs_metadata);

    for entry in try_io!(blob_id, archive.entries()) {
        event_handler.layer_progress(tarball_position.get());
        ctx.unpack(entry)?;
    }

    event_handler.layer_progress(tarball_position.get());

    Ok(())
}

struct InvalidEntryType(tar::EntryType);

impl std::fmt::Display for InvalidEntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid entry type: {:?}", self.0)
    }
}

/// Count how many bytes have been read from `reader`.
struct PositionTracker<'a, R> {
    count: &'a Cell<usize>,
    reader: R,
}

impl<T: Read> Read for PositionTracker<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        self.count.set(self.count.get() + n);
        Ok(n)
    }
}

struct Context<'a, E> {
    event_handler: &'a E,
    blob_id: &'a str,
    target: &'a Directory,
    dirs_cache: DirFdCache<'a>,
    dirs_metadata: &'a mut DirectoryMetadata,
    cached_link_dirfd: Option<(PathBuf, OwnedFd)>,
}

impl<'a, E: EventHandler> Context<'a, E> {
    fn new(
        event_handler: &'a E,
        blob_id: &'a str,
        target: &'a Directory,
        dirs_metadata: &'a mut DirectoryMetadata,
    ) -> Self {
        Self {
            event_handler,
            blob_id,
            target,
            dirs_cache: DirFdCache::new(target),
            dirs_metadata,
            cached_link_dirfd: None,
        }
    }

    fn path_fd(&mut self, path: impl AsRef<Path>) -> io::Result<BorrowedFd<'_>> {
        Ok(self.dirs_cache.get(path, true)?)
    }

    fn unpack(&mut self, entry: io::Result<tar::Entry<impl Read>>) -> Result<(), UnpackError> {
        let entry = try_io!(self.blob_id, entry);

        let entry_path = try_io!(self.blob_id, entry.path());
        let entry_path = entry_path.as_ref();

        let (parent_path, file_name) = try_io!(entry_path, normalize_path(entry_path));

        // Handle whiteout entries.
        //
        // The cache for directory file descriptors is reset after removing
        // any entry.
        if let Some(whiteout) = file_name
            .as_os_str()
            .as_bytes()
            .strip_prefix(WHITEOUT_PREFIX)
        {
            let parent_fd = try_io!(parent_path, self.path_fd(&parent_path));
            try_io!(entry_path, Self::process_whiteout(parent_fd, whiteout));
            self.dirs_cache.clear();
            return Ok(());
        }

        // Unpack the entry.
        try_io!(file_name, {
            match entry.header().entry_type() {
                tar::EntryType::Directory => self.unpack_dir(parent_path, &file_name, entry)?,

                tar::EntryType::Regular => self.unpack_regular(parent_path, &file_name, entry)?,

                tar::EntryType::Symlink | tar::EntryType::Link => {
                    self.unpack_link(self.target.as_fd(), parent_path, &file_name, entry)?
                }

                other => {
                    self.event_handler
                        .layer_entry_skipped(entry.path()?.as_ref(), &InvalidEntryType(other));
                }
            }
        });

        Ok(())
    }

    fn unpack_dir(
        &mut self,
        parent_path: impl AsRef<Path>,
        file_name: &Path,
        entry: tar::Entry<impl Read>,
    ) -> io::Result<()> {
        use rustix::fs;

        let header = entry.header();

        let parent_fd = self.path_fd(parent_path)?;
        let result = fs::mkdirat(parent_fd, file_name, Mode::from_raw_mode(0o700));

        if let Err(e) = result {
            // Ignore the error if the directory already exists.
            if e.kind() == AlreadyExists && !Self::is_directory(parent_fd, file_name)? {
                return Err(e.into());
            }
        }

        let (uid, gid) = Self::get_entry_owner(header)?;

        // Store mtime/mode metadata to be applied later.
        if let Ok(mtime) = header.mtime() {
            let (mut path, b) = normalize_path(entry.path()?)?;
            path.push(b);

            let key = super::DirectoryMetadataEntry::key(path);
            let entry = super::DirectoryMetadataEntry {
                mode: Mode::from_bits_retain(header.mode()?),
                mtime,
                uid,
                gid,
            };

            self.dirs_metadata.insert(key, entry);
        }

        Ok(())
    }

    fn unpack_regular(
        &mut self,
        parent_path: impl AsRef<Path>,
        file_name: &Path,
        mut entry: tar::Entry<impl Read>,
    ) -> io::Result<()> {
        use rustix::fs;

        let mode = Mode::from_bits_retain(entry.header().mode()? & 0o7777);

        let parent_path = parent_path.as_ref();
        let parent_fd = self.dirs_cache.get(parent_path, true)?;

        let mut output = loop {
            let result = fs::openat2(
                parent_fd,
                file_name,
                fs::OFlags::CREATE | fs::OFlags::EXCL | fs::OFlags::WRONLY,
                mode,
                fs::ResolveFlags::BENEATH,
            );

            match result {
                Ok(f) => break File::from(f),

                Err(e) if e.kind() == AlreadyExists => {
                    let removed = crate::fs::remove_entry(parent_fd, file_name)?;

                    // Remove the entry from dirs_metadata if the entry
                    // was a directory.
                    if removed == crate::fs::RemovedEntry::Directory {
                        let path = parent_path.join(file_name);
                        let key = super::DirectoryMetadataEntry::key(path);
                        self.dirs_metadata.remove(&key);
                    }
                }

                Err(e) => return Err(e.into()),
            }
        };

        io::copy(&mut entry, &mut output)?;

        drop(output);

        Self::set_owner(parent_fd, file_name, entry.header())?;

        let mtime = Self::make_timestamps(entry.header().mtime()?);
        fs::utimensat(parent_fd, file_name, &mtime, fs::AtFlags::SYMLINK_NOFOLLOW)?;

        Ok(())
    }

    /// Unpack hard and symbolic links.
    fn unpack_link(
        &mut self,
        root_fd: BorrowedFd,
        parent_path: impl AsRef<Path>,
        file_name: &Path,
        entry: tar::Entry<impl Read>,
    ) -> io::Result<()> {
        use rustix::fs;

        let dest = match entry.link_name()? {
            Some(dest) => dest,
            None => return Err(io::Error::new(NotFound, "Missing link")),
        };

        let parent_fd = self.dirs_cache.get(parent_path, true)?;

        let is_symlink = entry.header().entry_type().is_symlink();

        loop {
            let result = if is_symlink {
                fs::symlinkat(dest.as_ref(), parent_fd, file_name)
            } else {
                // To create hard-links, get a file descriptor of the directory of
                // the source (`old_path`). The descriptor is cached because some OCI
                // images have multiple consecutive links in the same directory.

                let (old_parent, old_name) = normalize_path(&dest)?;

                let old_dirfd = match self.cached_link_dirfd.take() {
                    Some((cached_path, fd)) if cached_path == old_parent => fd,

                    _ => fs::openat2(
                        root_fd,
                        &old_parent,
                        fs::OFlags::PATH | fs::OFlags::NOFOLLOW,
                        fs::Mode::empty(),
                        fs::ResolveFlags::IN_ROOT | fs::ResolveFlags::NO_MAGICLINKS,
                    )?,
                };

                let result = fs::linkat(
                    &old_dirfd,
                    old_name,
                    parent_fd,
                    file_name,
                    fs::AtFlags::empty(),
                );

                self.cached_link_dirfd = Some((old_parent, old_dirfd));

                result
            };

            match result {
                Ok(_) => break,

                Err(e) if e.kind() == AlreadyExists => {
                    fs::unlinkat(parent_fd, file_name, fs::AtFlags::empty())?;
                }

                Err(e) => return Err(e.into()),
            }
        }

        if is_symlink {
            let header = entry.header();

            let mtime = Self::make_timestamps(header.mtime()?);
            fs::utimensat(parent_fd, file_name, &mtime, fs::AtFlags::SYMLINK_NOFOLLOW)?;

            Self::set_owner(parent_fd, file_name, header)?;
        }

        Ok(())
    }

    fn is_directory(parent: BorrowedFd, file_name: &Path) -> io::Result<bool> {
        let stat = rustix::fs::statat(parent, file_name, rustix::fs::AtFlags::empty())?;
        Ok(stat.st_mode & libc::S_IFDIR != 0)
    }

    fn make_timestamps(mtime: u64) -> rustix::fs::Timestamps {
        let mtime = rustix::fs::Timespec {
            tv_sec: i64::try_from(mtime).unwrap_or_default(),
            tv_nsec: 0,
        };

        rustix::fs::Timestamps {
            last_access: mtime,
            last_modification: mtime,
        }
    }

    /// Return the `uid, gid` of the entry.
    ///
    /// The owner is ignored if it is `0` (`root`).
    fn get_entry_owner(header: &tar::Header) -> io::Result<(Option<u32>, Option<u32>)> {
        Ok((
            match header.uid().map(|id| id.try_into()) {
                Ok(Ok(id)) if id > 0 => Some(id),
                _ => None,
            },
            match header.gid().map(|id| id.try_into()) {
                Ok(Ok(id)) if id > 0 => Some(id),
                _ => None,
            },
        ))
    }

    /// Set user/group of an entry.
    ///
    /// Errors from `fchownat` are ignored.
    fn set_owner(parent_fd: BorrowedFd, file_name: &Path, header: &tar::Header) -> io::Result<()> {
        let (uid, gid) = Self::get_entry_owner(header)?;
        crate::fs::change_owner(parent_fd, file_name, uid, gid, true)?;
        Ok(())
    }

    /// Process whiteout entries, by removing files in the directory.
    ///
    /// The [1]specification indicates that whiteout entries _should_
    /// appear before regular files. This implementation assumes that
    /// the layer was built in such way.
    ///
    /// [1]: https://github.com/opencontainers/image-spec/blob/v1.0/layer.md#whiteouts
    fn process_whiteout(dir: BorrowedFd, whiteout: &[u8]) -> io::Result<()> {
        let path = Path::new(OsStr::from_bytes(whiteout));

        if whiteout == WHITEOUT_OPAQUE {
            crate::fs::remove_subtree(dir, Path::new("."))?;
        } else {
            crate::fs::remove_entry(dir, path)?;
        }

        Ok(())
    }
}
