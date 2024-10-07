mod manifests;

mod extractor;
mod layers;

use std::io;
use std::path::{Path, PathBuf};

use crate::{digest::DigestParseError, reference::Reference, MediaType};

/// Handler to receive notification for events during the download process.
#[expect(unused_variables)]
pub trait EventHandler: Send + Sync + 'static {
    fn registry_request(&self, url: &str) {}

    fn registry_auth(&self, url: &str) {}

    fn download_start(&self, layers: usize, bytes: usize) {}

    fn download_progress_bytes(&self, bytes: usize) {}

    fn layer_start(&self, archive_len: u64) {}

    fn layer_progress(&self, position: usize) {}

    fn finished(&self) {}

    #[cfg(feature = "sandbox")]
    fn sandbox_status(&self, status: landlock::RestrictionStatus) {}
}

#[derive(thiserror::Error, Debug)]
pub enum UnpackError {
    #[error("I/O error in {1}: {0}")]
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
    InvalidDigest(#[from] DigestParseError),

    #[error("Missing or invalid Content-Type.")]
    MissingContentType,

    #[error("Invalid Content-Type: {0}")]
    InvalidContentType(MediaType),

    #[error("No image for the architecture.")]
    MissingArchitecture,
}

macro_rules! try_io {
    ($path:expr, $e:expr $(,)?) => {
        $e.map_err(|e| UnpackError::Io(io::Error::from(e), $path.into()))?
    };
}

// Make visible to mods.
use try_io;

/// Download the image of `reference`, and extract its contents to the
/// directory `target`.
///
/// If `target` exists, it must be empty.
///
/// Before unpacking the layers, it tries to create a sandbox to restrict
/// the write access to the `target` directory. If the sandbox can't be
/// created, and `require_sandbox` is `true`, the process is interrupted.
pub fn unpack(
    reference: &Reference,
    architecture: Option<&str>,
    os: Option<&str>,
    event_handler: impl EventHandler,
    target: &Path,
    require_sandbox: bool,
) -> Result<(), UnpackError> {
    check_empty_dir(target).map_err(|e| UnpackError::Io(e, target.to_owned()))?;

    let mut client = crate::http::Client::new(reference, &event_handler);

    let manifest = manifests::get(reference, architecture, os, &mut client)?;

    // Create sandbox after downloading the manifest, but before writing any
    // file. Thus, we don't need to gran read-access to the files needed to
    // make HTTPS requests (like `/etc/resolv.conf` or `/etc/ssl`).
    #[cfg(feature = "sandbox")]
    if let Err(err) = sandbox(target, &event_handler) {
        if require_sandbox {
            return Err(UnpackError::Sandbox(err));
        }
    }

    layers::get(client, manifest, target, &event_handler)
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
fn sandbox(target: &Path, event_handler: &impl EventHandler) -> Result<(), landlock::RulesetError> {
    use landlock::*;

    let abi = ABI::V3;

    let status = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(abi))?
        .create()?
        .add_rules(path_beneath_rules(&[target], AccessFs::from_all(abi)))?
        .restrict_self()?;

    event_handler.sandbox_status(status);

    Ok(())
}
