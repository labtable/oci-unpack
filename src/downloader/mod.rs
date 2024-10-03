mod manifests;

mod http;
mod mime;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use crate::reference::Reference;

/// Handle to receive notification for events during the download process.
#[expect(unused_variables)]
pub trait EventHandler: Send + 'static {
    fn registry_request(&self, url: &str) {}

    fn registry_auth(&self, url: &str) {}
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadError {
    #[error("HTTP request failed")]
    HttpRequest(#[from] Box<ureq::Error>),

    #[error("Invalid JSON")]
    Json(#[from] serde_json::Error),

    #[error("Missing authentication tokens.")]
    MissingTokens,

    #[error("Missing or invalid Content-Type.")]
    InvalidContentType,

    #[error("No image for the architecture.")]
    MissingArchitecture,
}

impl From<ureq::Error> for DownloadError {
    fn from(value: ureq::Error) -> Self {
        DownloadError::HttpRequest(Box::new(value))
    }
}

/// Download the image described in `reference` to the directory `target`.
///
/// `target` is expected to not exist, or to be empty.
pub fn download(
    reference: &Reference,
    architecture: Option<&str>,
    os: Option<&str>,
    event_handler: impl EventHandler,
    target: PathBuf,
) -> Result<(), DownloadError> {
    let mut client = http::Client::new(reference.registry, event_handler);

    let manifest = manifests::get(reference, architecture, os, &mut client)?;

    todo!();
}
