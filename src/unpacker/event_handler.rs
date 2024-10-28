use std::{fmt::Display, path::Path};

/// Handler to receive notifications for events during the unpack process.
///
/// All methods are optional.
#[expect(unused_variables)]
pub trait EventHandler: Sync + 'static {
    /// HTTP request to the registry.
    fn registry_request(&self, url: &str) {}

    /// Registry requires an [authentication token][token].
    ///
    /// [token]: https://distribution.github.io/distribution/spec/auth/token/
    fn registry_auth(&self, url: &str) {}

    /// Start to download the blobs of the image.
    ///
    /// `layers` is the number of layers to download.
    ///
    /// `bytes` is the size of the data that is going to be downloaded.
    fn download_start(&self, layers: usize, bytes: usize) {}

    /// Some data (in `bytes`) has been received.
    ///
    /// This method is invoked very frequently.
    fn download_progress_bytes(&self, bytes: usize) {}

    /// Start to unpack a downloaded layer.
    ///
    /// `archive_length` is the length, in bytes, of the archive
    /// containing the layer.
    fn layer_start(&self, archive_length: u64) {}

    /// A file was extracted from the archive and was written to the disk.
    ///
    /// `archive_position` is relative to `archive_length` in
    /// [`layer_start`][Self::layer_start].
    /// When `archive_position == archive_length`, the layer is fully
    /// unpacked.
    fn layer_progress(&self, archive_position: usize) {}

    /// An entry in the archive's layer is skipped.
    ///
    /// For example, if it is an invalid entry type, like a block device.
    fn layer_entry_skipped(&self, path: &Path, cause: &dyn Display) {}

    /// All layers have been unpacked.
    fn finished(&self) {}

    #[cfg(feature = "sandbox")]
    fn sandbox_status(&self, status: landlock::RestrictionStatus) {}
}

/// [`EventHandler`] instance to ignore all events.
pub struct NoEventHandler;

impl EventHandler for NoEventHandler {}
