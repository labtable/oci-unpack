//! Parse a reference to an image in an OCI registry.
//!
//! It tries to be close to what `docker pull` does, but it does not try
//! to be bug-for-bug compatible.

use super::*;

/// Hostname to use when the reference is just the repository,
/// like `debian` or `nixos/nix`.
const DEFAULT_REGISTRY: &str = "registry-1.docker.io";

const DEFAULT_NAMESPACE: &str = "library";

const DEFAULT_TAG: &str = "latest";

type Result<T> = std::result::Result<T, ParseError>;

pub(super) fn parse(reference: &str) -> Result<Reference<'_>> {
    // Extract the digest after the last `@`.
    let (base, digest) = match reference.rsplit_once("@") {
        None => (reference, None),
        Some((base, d)) => (base, Some(Digest::try_from(d.to_owned())?)),
    };

    // Extract the tag after the last `:`.
    //
    // If the value contains a `/`, it assumes that the value after `:`
    // is a port number, and not a tag.
    let (base, tag) = match base.rsplit_once(":") {
        Some((base, tag)) if !tag.contains('/') => (base, tag),
        _ => (base, DEFAULT_TAG),
    };

    // Imitate the logic from `docker pull` to get the repository.
    let (registry, repository) = match base.split_once('/') {
        // There is no `/`. The reference is an image in the
        // `library` repository.
        None => (
            DEFAULT_REGISTRY,
            Repository::components(DEFAULT_NAMESPACE, base),
        ),

        // There is a `.` before the `/`. Parse it as a hostname.
        Some((registry, repository)) if registry.contains('.') => {
            (registry, Repository::full(repository))
        }

        // There is no `.`. Assume it is a repository in the default registry.
        Some(_) => (DEFAULT_REGISTRY, Repository::full(base)),
    };

    if repository.name().is_empty() {
        return Err(ParseError::MissingRepository);
    }

    Ok(Reference {
        registry,
        repository,
        tag,
        digest,
    })
}

#[test]
fn parse_valid_references() {
    use crate::digest::HexString;
    use sha2::{Digest as _, Sha256, Sha512};

    macro_rules! check {
        ($reference:expr, [ $registry:expr, $repository:expr, $tag:expr, $digest:expr ]) => {
            let reference = $reference;
            assert_eq!(
                Reference::try_from(<_ as AsRef<str>>::as_ref(&reference)).unwrap(),
                Reference {
                    registry: $registry,
                    repository: $repository,
                    tag: $tag,
                    digest: $digest,
                }
            )
        };
    }

    let sha256 = HexString(Sha256::digest(b"\x00\x01"));
    let sha512 = HexString(Sha512::digest(b"\x01\x02"));

    check!(
        "foo",
        [
            DEFAULT_REGISTRY,
            Repository::components("library", "foo"),
            DEFAULT_TAG,
            None
        ]
    );

    check!(
        "foo/bar",
        [
            DEFAULT_REGISTRY,
            Repository::full("foo/bar"),
            DEFAULT_TAG,
            None
        ]
    );

    check!(
        "example.com:5678/foo/bar:1.2.3",
        [
            "example.com:5678",
            Repository::full("foo/bar"),
            "1.2.3",
            None
        ]
    );

    check!(
        &format!("example.com/foo/bar:1.2.3@sha256:{sha256}"),
        [
            "example.com",
            Repository::full("foo/bar"),
            "1.2.3",
            Digest::try_from(format!("sha256:{sha256}")).ok()
        ]
    );

    check!(
        &format!("example.com:1234/foo/bar:1.2.3@sha512:{sha512}"),
        [
            "example.com:1234",
            Repository::full("foo/bar"),
            "1.2.3",
            Digest::try_from(format!("sha512:{sha512}")).ok()
        ]
    );
}

#[test]
fn reject_invalid_digests() {
    use crate::digest::DigestError;

    assert!(matches!(
        Reference::try_from("debian:stable@md5:0000"),
        Err(ParseError::InvalidDigest(DigestError::InvalidAlgorithm)),
    ));

    assert!(matches!(
        Reference::try_from("debian:stable@sha256:0000"),
        Err(ParseError::InvalidDigest(DigestError::InvalidValue)),
    ));

    assert!(matches!(
        Reference::try_from(format!("debian:stable@sha256:{:064}", "x").as_str()),
        Err(ParseError::InvalidDigest(DigestError::InvalidValue)),
    ));
}
