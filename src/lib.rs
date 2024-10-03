#![expect(dead_code)]

mod digest;
mod downloader;

pub mod reference;

pub use downloader::{download, EventHandler};
pub use reference::Reference;
