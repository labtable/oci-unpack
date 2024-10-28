mod mediatype;
mod parser;

use crate::digest::Digest;

pub use mediatype::MediaType;

/// Errors from [`Reference::try_from`].
#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("Missing repository.")]
    MissingRepository,

    #[error("{0}")]
    InvalidDigest(#[from] crate::digest::DigestError),
}

/// Reference to an image in an OCI registry.
///
/// The parser tries to be close to what `docker pull` does:
///
/// * If the reference does not include the hostname of the registry,
///   it uses Docker Hub, and the repository namespace defaults to
///   `library` if there is none. For example:
///
///   * `debian` is parsed as `registry-1.docker.io/library/debian`.
///   * `nixos/nix` is parsed as `registry-1.docker.io/nixos/nix`.
/// * It accepts any tag value after the last `:` character. If no tag
///   is given, it uses `latest`.
/// * It accepts a fixed digest (the last part after a `@` character), but
///   only SHA256 and SHA512.
///
/// However, it does not try to be bug-for-bug compatible with Docker.
///
/// # Examples
///
/// ```
/// # use oci_unpack::*;
/// const REFERENCE: &str = "registry.example.com/foo/bar:1.23.4@sha256:123456789012345678901234567890123456789012345678901234567890ABCD";
///
/// let reference = Reference::try_from(REFERENCE).unwrap();
/// assert_eq!(reference.registry, "registry.example.com");
/// assert_eq!(reference.repository.namespace(), Some("foo"));
/// assert_eq!(reference.repository.name(), "bar");
/// assert_eq!(reference.tag, "1.23.4");
///
/// let digest = reference.digest.as_ref().unwrap();
/// assert_eq!(digest.algorithm(), DigestAlgorithm::SHA256);
/// assert_eq!(digest.hash_value(), "123456789012345678901234567890123456789012345678901234567890ABCD");
/// ```
///
/// ```
/// # use oci_unpack::*;
/// let reference = Reference::try_from("debian:stable").unwrap();
///
/// assert_eq!(reference.repository.to_string(), "library/debian");
/// assert_eq!(reference.tag, "stable");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Reference<'a> {
    /// Address of the registry server.
    pub registry: &'a str,

    /// Repository name.
    pub repository: Repository<'a>,

    /// Image tag.
    pub tag: &'a str,

    /// Manifest digest, if present.
    pub digest: Option<Digest>,
}

/// Represents a repository name, like `library/debian`
/// or `nixos/nix`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Repository<'a>(RepositoryInner<'a>);

impl<'a> Repository<'a> {
    pub(crate) fn components(namespace: &'a str, name: &'a str) -> Self {
        Repository(RepositoryInner::Components(namespace, name))
    }

    pub(crate) fn full(name: &'a str) -> Self {
        Repository(RepositoryInner::Full(name))
    }

    /// Return the name of this repository.
    ///
    /// # Examples
    ///
    /// ```
    /// # use oci_unpack::*;
    /// let reference = Reference::try_from("foo/bar:stable").unwrap();
    /// assert_eq!(reference.repository.name(), "bar");
    /// ```
    pub fn name(&self) -> &str {
        match self.0 {
            RepositoryInner::Full(full) => full.split_once('/').map(|s| s.1).unwrap_or(full),
            RepositoryInner::Components(_, name) => name,
        }
    }

    /// Return the namespace of this repository, or `None` if
    /// the repository does not contain a `/` character.
    ///
    /// # Examples
    ///
    /// ```
    /// # use oci_unpack::*;
    /// let reference = Reference::try_from("foo/bar:stable").unwrap();
    /// assert_eq!(reference.repository.namespace(), Some("foo"));
    /// ```
    pub fn namespace(&self) -> Option<&str> {
        match self.0 {
            RepositoryInner::Full(full) => full.split_once('/').map(|s| s.0),
            RepositoryInner::Components(ns, _) => Some(ns),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum RepositoryInner<'a> {
    /// Full repository name. Namespace is optional.
    Full(&'a str),

    /// Namespace and name.
    Components(&'a str, &'a str),
}

impl<'a> std::fmt::Display for Repository<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            RepositoryInner::Full(full) => f.write_str(full),
            RepositoryInner::Components(a, b) => write!(f, "{a}/{b}"),
        }
    }
}

impl<'a> TryFrom<&'a str> for Reference<'a> {
    type Error = ParseError;

    fn try_from(reference: &'a str) -> Result<Self, Self::Error> {
        parser::parse(reference)
    }
}
