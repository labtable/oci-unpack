use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{openat, Mode, OFlags},
    io::Errno,
    path::Arg,
};

pub(crate) struct Directory {
    fd: OwnedFd,
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
        openat(self.fd.as_fd(), path, OFlags::CREATE | OFlags::WRONLY, mode)
    }

    /// Create and unnamed temporary regular file.
    pub fn tmpfile(&self) -> Result<OwnedFd, Errno> {
        openat(
            self.fd.as_fd(),
            c".",
            OFlags::TMPFILE | OFlags::RDWR | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )
    }
}
