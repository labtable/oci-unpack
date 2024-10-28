use std::{
    io,
    num::NonZeroUsize,
    path::{Component, Path, PathBuf},
};

use rustix::{
    fd::{AsFd, BorrowedFd, OwnedFd},
    fs::{
        chmodat, chownat, mkdirat, openat, openat2, AtFlags, Gid, Mode, OFlags, ResolveFlags, Uid,
    },
    io::Errno,
    path::Arg,
};

/// Provides some functions to create files and directories under a specific path.
///
/// It relies on a file descriptor to ensure that new entries are never created
/// outside the root.
pub(crate) struct Directory {
    fd: OwnedFd,
}

impl From<OwnedFd> for Directory {
    fn from(fd: OwnedFd) -> Self {
        Directory { fd }
    }
}

impl AsFd for Directory {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl Directory {
    pub fn new<P: Arg>(target: P) -> Result<Self, Errno> {
        let fd = openat(
            rustix::fs::CWD,
            target,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;

        Ok(Directory { fd })
    }

    /// Create a new file in this directory.
    pub fn create<P: Arg>(&self, path: P, mode: Mode) -> Result<OwnedFd, Errno> {
        openat2(
            self,
            path,
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY,
            mode,
            ResolveFlags::BENEATH,
        )
    }

    /// Create and unnamed temporary regular file.
    pub fn tmpfile(&self) -> Result<OwnedFd, Errno> {
        openat(
            self,
            c".",
            OFlags::TMPFILE | OFlags::RDWR | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )
    }

    /// Return a file descriptor for a directory.
    ///
    /// If `create` is `true`, the directory is created if it does not exist.
    pub fn open_directory<P>(&self, path: P, create: bool) -> Result<OwnedFd, Errno>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();

        loop {
            let result = openat2(
                self,
                path,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::IN_ROOT | ResolveFlags::NO_MAGICLINKS,
            );

            match result {
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => (),
                r => return r,
            }

            // At this point, the directory does not exist, and we want
            // to create it.

            let file_name = match path.file_name() {
                Some(f) => f,
                None => return Err(Errno::NOENT),
            };

            // Get a FD to the parent to use `mkdirat`. This is needed to be
            // able to rely on `RESOLVE_IN_ROOT` to resolve symlinks inside
            // our own root.
            let owned_slot;
            let parent = match path.parent() {
                Some(p) if p == Path::new("") => &self.fd,

                None => &self.fd,

                Some(p) => {
                    owned_slot = self.open_directory(p, create)?;
                    &owned_slot
                }
            };

            mkdirat(parent.as_fd(), file_name, Mode::from_raw_mode(0o755))?;
        }
    }
}

/// LRU cache of file descriptors for directories.
pub(crate) struct DirFdCache<'a> {
    directory: &'a Directory,
    cache: lru::LruCache<PathBuf, OwnedFd>,
}

/// Number of entries in a file descriptor cache.
const FDS_CACHE: usize = 16;

impl<'a> DirFdCache<'a> {
    pub fn new(directory: &'a Directory) -> Self {
        let cache = lru::LruCache::new(NonZeroUsize::new(FDS_CACHE).unwrap());
        DirFdCache { directory, cache }
    }

    /// Get a file descriptor for a directory.
    pub fn get<P>(&mut self, path: P, create: bool) -> Result<BorrowedFd, Errno>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();

        self.cache
            .try_get_or_insert_ref(path, || self.directory.open_directory(path, create))
            .map(|fd| fd.as_fd())
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

/// Convert a path from an archive entry to the expected path inside
/// a container.
///
/// The path is returned in a `(parent, file_name)` pair.
///
/// The parent is always prefixed with `/`.
pub fn normalize_path<T: AsRef<Path>>(path: T) -> io::Result<(PathBuf, PathBuf)> {
    let mut parent_path = PathBuf::from("/");
    let mut file_name = None;

    // Similar to `tar::Entry::unpack_in`.
    for component in path.as_ref().components() {
        match component {
            Component::Prefix(..) | Component::RootDir | Component::CurDir => continue,

            // Don't trust entries with `..` in the path.
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Found '..' in the path.",
                ));
            }

            Component::Normal(part) => {
                if let Some(previous) = file_name.take() {
                    parent_path.push(previous);
                }

                file_name = Some(part)
            }
        }
    }

    let file_name = match file_name {
        Some(file_name) => PathBuf::from(file_name),

        None => PathBuf::from("."),
    };

    Ok((parent_path, file_name))
}

/// Change the owner of an entry.
///
/// If `preserve_mode` is `true`, the mode will be restored if it
/// contains any SUID flag.
///
/// Errors from `fchownat` are ignored.
pub fn change_owner(
    parent_fd: BorrowedFd,
    file_name: &Path,
    uid: Option<u32>,
    gid: Option<u32>,
    preserve_mode: bool,
) -> io::Result<()> {
    if uid.is_none() && gid.is_none() {
        return Ok(());
    }

    let orig_mode = if preserve_mode {
        Some(rustix::fs::statat(parent_fd, file_name, AtFlags::SYMLINK_NOFOLLOW)?.st_mode)
    } else {
        None
    };

    let result = chownat(
        parent_fd,
        file_name,
        uid.map(|id| unsafe { Uid::from_raw(id) }),
        gid.map(|id| unsafe { Gid::from_raw(id) }),
        AtFlags::SYMLINK_NOFOLLOW,
    );

    if let (Ok(_), Some(mode)) = (result, orig_mode) {
        // Restore SUID bits, if any.
        if mode & 0o7000 != 0 {
            chmodat(
                parent_fd,
                file_name,
                Mode::from_bits_retain(mode),
                AtFlags::empty(),
            )?;
        }
    }

    Ok(())
}
