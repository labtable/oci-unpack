//! Parse a reference to an image in an OCI registry.
//!
//! It tries to be close to what `docker pull` does, but it does not try
//! to be bug-for-bug compatible.

use super::*;

/// Hostname to use when the reference is only the repository,
/// like `debian` or `nixos/nix`.
const DEFAULT_REGISTRY: &str = "registry-1.docker.io";

/// Repository prefix when it does not contain an `/`.
const DEFAULT_REPOSITORY: &str = "library";

const DEFAULT_TAG: &str = "latest";

type Result<T> = std::result::Result<T, ParseError>;

pub(super) fn parse(reference: &str) -> Result<Reference<'_>> {
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
            Repository::Prefixed(DEFAULT_REPOSITORY, base),
        ),

        // There is a `.` before the `/`. Parse it as a hostname.
        Some((registry, repository)) if registry.contains('.') => {
            (registry, Repository::Full(repository))
        }

        // There is no `.` Assume it is a repository in the default registry.
        Some(_) => (DEFAULT_REGISTRY, Repository::Full(base)),
    };

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
            assert_eq!(
                Reference::parse($reference).unwrap(),
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
            Repository::Prefixed("library", "foo"),
            DEFAULT_TAG,
            None
        ]
    );

    check!(
        "foo/bar",
        [
            DEFAULT_REGISTRY,
            Repository::Full("foo/bar"),
            DEFAULT_TAG,
            None
        ]
    );

    check!(
        "example.com:5678/foo/bar:1.2.3",
        [
            "example.com:5678",
            Repository::Full("foo/bar"),
            "1.2.3",
            None
        ]
    );

    check!(
        &format!("example.com/foo/bar:1.2.3@sha256:{sha256}"),
        [
            "example.com",
            Repository::Full("foo/bar"),
            "1.2.3",
            Digest::try_from(format!("sha256:{sha256}")).ok()
        ]
    );

    check!(
        &format!("example.com:1234/foo/bar:1.2.3@sha512:{sha512}"),
        [
            "example.com:1234",
            Repository::Full("foo/bar"),
            "1.2.3",
            Digest::try_from(format!("sha512:{sha512}")).ok()
        ]
    );
}

#[test]
fn reject_invalid_digests() {
    use crate::digest::DigestParseError;

    assert!(matches!(
        Reference::parse("debian:stable@md5:0000"),
        Err(ParseError::Digest(DigestParseError::InvalidDigestAlgorithm)),
    ));

    assert!(matches!(
        Reference::parse("debian:stable@sha256:0000"),
        Err(ParseError::Digest(DigestParseError::InvalidDigest)),
    ));

    assert!(matches!(
        Reference::parse(&format!("debian:stable@sha256:{:064}", "x")),
        Err(ParseError::Digest(DigestParseError::InvalidDigest)),
    ));
}
