//! This crate implements the basic support to download and unpack
//! [OCI images](https://github.com/opencontainers/image-spec) stored
//! in a [container registry](https://distribution.github.io/distribution/).
//!
//! It is not expected to support every feature in the OCI specifications. Instead,
//! the goal is to implement all features used in the most common images.
//!
//! # Usage
//!
//! The first step for unpacking an OCI image is to get a [reference][Reference]
//! instance to describe its location:
//!
//! ```
//! # use oci_unpack::*;
//! let reference = Reference::try_from("debian:stable").unwrap();
//! ```
//!
//! The string is parsed following the same rules as the `docker pull` command,
//! as described in the [`Reference`] documentation.
//!
//! Then, an [`Unpacker`] instance is created to configure how to download and
//! unpack the referenced image.
//!
//! ```
//! # use oci_unpack::*;
//! # fn f(reference: Reference) {
//! Unpacker::new(reference).unpack("/tmp/image").unwrap();
//! # }
//! ```
//!
//! An instance of [`EventHandler`] can be used to receive notifications during
//! the download/unpack process. The file `examples/unpack.rs` in the repository
//! has a full implementation of a handler.
//!
//!
//! # Sandbox
//!
//! Before creating any file in the target directory, [`Unpacker::unpack`] tries
//! to create a sandbox with [Landlock](https://landlock.io/), so the process will
//! be able to create files only beneath the target directory.
//!
//! Errors on creating the sandbox can be ignored by setting [`Unpacker::require_sandbox`]
//! to `false`.
//!
//! The sandbox is only available if the crate is built with the `sandbox` feature, which
//! is enabled by default.

mod digest;
mod fs;
mod http;
mod manifests;
mod reference;
mod unpacker;

pub use digest::{Digest, DigestAlgorithm};
pub use reference::{MediaType, Reference, Repository};
pub use unpacker::{EventHandler, NoEventHandler, Unpacker};

/// Errors from the functions in the public API.
pub mod errors {
    pub use super::digest::DigestError;
    pub use super::reference::ParseError;
    pub use super::unpacker::UnpackError;
}
