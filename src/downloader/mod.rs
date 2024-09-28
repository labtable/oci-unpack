#[cfg(test)]
mod tests;
mod http;

#[derive(thiserror::Error, Debug)]
pub enum DownloadError {
    #[error("HTTP request failed")]
    HttpRequest(#[from] ureq::Error),

    #[error("Invalid JSON")]
    Json(#[from] serde_json::Error),

    #[error("Missing authentication tokens.")]
    MissingTokens,
}

/// Download the image described in `reference` to the directory `target`.
///
/// `target` is expected to not exist, or to be empty.
pub fn download(reference: &Reference, target: PathBuf) -> std::io::Result<()> {
    todo!()
}
