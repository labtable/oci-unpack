mod event_handler;
mod images;
mod layers;

use std::collections::BTreeMap;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::{digest::DigestError, reference::Reference, MediaType};

pub use event_handler::{EventHandler, NoEventHandler};

/// Errors from [`Unpacker::unpack`].
#[derive(thiserror::Error, Debug)]
pub enum UnpackError {
    #[error("I/O error: {1}: {0}")]
    Io(io::Error, PathBuf),

    #[cfg(feature = "sandbox")]
    #[error("Failed to create a sandbox: {0}")]
    Sandbox(#[from] landlock::RulesetError),

    #[error("Operation interrupted.")]
    Interrupted,

    #[error("HTTP request failed: {0}")]
    HttpRequest(#[from] crate::http::HttpError),

    #[error("Invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid digest: {0}")]
    InvalidDigest(#[from] DigestError),

    #[error("Missing or invalid Content-Type.")]
    MissingContentType,

    #[error("Invalid Content-Type: {0}")]
    InvalidContentType(MediaType),

    #[error("No image for the architecture.")]
    MissingArchitecture,
}

/// Wrap a [std::io::Error] with the path related to the I/O operation.
///
/// The second argument can be either a single expression, or a block.
macro_rules! try_io {
    ($path:expr, $b:block) => {
        match (|| -> Result<_, io::Error> { Ok($b) })() {
            Ok(ok) => ok,
            Err(err) => return Err(UnpackError::Io(io::Error::from(err), $path.into())),
        }
    };

    ($path:expr, $e:expr $(,)?) => {
        $e.map_err(|e| UnpackError::Io(io::Error::from(e), $path.into()))?
    };
}

// Make visible to mods.
use try_io;

/// Track directory metadata, to be applied when all files are written.
///
/// `mode` can't be set during the unpack, because if a directory has no
/// `write` or `execute` permission, the program can't create files.
///
/// `mtime` could be set, but it is replaced by the kernel when new files
/// are unpacked.
///
/// The `usize` field in the key is the length, in bytes, of the path. It
/// is needed to guarantee that child directories are updated before their
/// parents.
type DirectoryMetadata = BTreeMap<(usize, PathBuf), DirectoryMetadataEntry>;

struct DirectoryMetadataEntry {
    mode: rustix::fs::Mode,
    mtime: u64,
    uid: Option<u32>,
    gid: Option<u32>,
}

impl DirectoryMetadataEntry {
    /// Return a key to use with `DirectoryMetadata`
    fn key(path: PathBuf) -> (usize, PathBuf) {
        let path_len = path.as_os_str().as_bytes().len();
        (usize::MAX - path_len, path)
    }
}

/// Download an image and unpack its contents to a new directory.
pub struct Unpacker<'a, E> {
    reference: Reference<'a>,
    architecture: Option<&'a str>,
    os: Option<&'a str>,
    event_handler: E,
    require_sandbox: bool,
}

impl<'a> Unpacker<'a, NoEventHandler> {
    /// Create a new unpacker for the given reference.
    ///
    /// Sandbox is required by default.
    pub fn new(reference: Reference<'a>) -> Self {
        Self {
            reference,
            architecture: None,
            os: None,
            event_handler: NoEventHandler,
            require_sandbox: true,
        }
    }

    /// Set a handler to receive events during the operation.
    pub fn event_handler<E: EventHandler>(self, event_handler: E) -> Unpacker<'a, E> {
        Unpacker {
            event_handler,
            reference: self.reference,
            architecture: self.architecture,
            os: self.os,
            require_sandbox: self.require_sandbox,
        }
    }
}

impl<'a, E: EventHandler> Unpacker<'a, E> {
    /// Set sandbox requirement.
    ///
    /// If `require_sandbox` is `false`, the unpacker ignores errors if
    /// it can't create a sandbox to restrict filesystem access.
    pub fn require_sandbox(mut self, require_sandbox: bool) -> Self {
        self.require_sandbox = require_sandbox;
        self
    }

    /// Set the expected CPU architecture of the image.
    ///
    /// If omitted, it uses the architecture currently in use.
    pub fn architecture(mut self, architecture: &'a str) -> Self {
        self.architecture = Some(architecture);
        self
    }

    /// Set the expected operating system  the image.
    ///
    /// If omitted, it uses the operating system currently in use.
    pub fn os(mut self, os: &'a str) -> Self {
        self.os = Some(os);
        self
    }

    /// Download the image of `reference`, and unpack its contents to the
    /// directory `target`.
    ///
    /// If `target` exists, it must be empty.
    ///
    /// Before unpacking the layers, it tries to create a sandbox to restrict
    /// the write access to the `target` directory. If the sandbox can't be
    /// created, and `require_sandbox` is `true`, the process is interrupted.
    pub fn unpack(self, target: impl AsRef<Path>) -> Result<(), UnpackError> {
        let target = target.as_ref();

        Self::check_empty_dir(target).map_err(|e| UnpackError::Io(e, target.to_owned()))?;

        let mut client = crate::http::Client::new(&self.reference, &self.event_handler);

        let manifest =
            crate::manifests::get(&self.reference, self.architecture, self.os, &mut client)?;

        // Create sandbox after downloading the manifest, but before writing any
        // file. Thus, we don't need to gran read-access to the files needed to
        // make HTTPS requests (like `/etc/resolv.conf` or `/etc/ssl`).
        #[cfg(feature = "sandbox")]
        if let Err(err) = Self::sandbox(target, &self.event_handler) {
            if self.require_sandbox {
                return Err(UnpackError::Sandbox(err));
            }
        }

        images::get(client, manifest, target, &self.event_handler)
    }

    /// Check if the `target` directory is empty.
    ///
    /// The directory is created if it does not exist.
    fn check_empty_dir(path: &Path) -> io::Result<()> {
        if !path.exists() {
            return std::fs::create_dir_all(path);
        }

        if std::fs::read_dir(path)?.next().is_some() {
            // Use `ErrorKind::DirectoryNotEmpty` when the
            // feature `io_error_more` is stabilized.
            return Err(io::Error::from_raw_os_error(libc::ENOTEMPTY));
        }

        Ok(())
    }

    /// Restrict filesystem access to the `target` directory.
    ///
    /// The sandbox must be created after initializing the HTTP client,
    /// since the rules don't allow access to other files in the system,
    /// like `/etc/resolv.conf` or `/etc/ssl`.
    #[cfg(feature = "sandbox")]
    fn sandbox(
        target: &Path,
        event_handler: &impl EventHandler,
    ) -> Result<(), landlock::RulesetError> {
        use landlock::*;

        let abi = ABI::V2;

        let status = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(abi))?
            .create()?
            .add_rules(path_beneath_rules(&[target], AccessFs::from_all(abi)))?
            .restrict_self()?;

        event_handler.sandbox_status(status);

        Ok(())
    }
}
