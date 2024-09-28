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

type Result<'a, T> = std::result::Result<T, ParseError<'a>>;

pub(super) fn parse(reference: &str) -> Result<'_, Reference<'_>> {
    let (base, digest) = extract_digest(reference)?;

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

/// Extract the digest after the last `@` in the reference.
fn extract_digest(reference: &str) -> Result<'_, (&str, Option<Digest<'_>>)> {
    match reference.rsplit_once("@") {
        None => Ok((reference, None)),

        Some((base, suffix)) => {
            let (digest, value, expected_size) = {
                if let Some(d) = suffix.strip_prefix("sha256:") {
                    (Digest::SHA256(d), d, 64)
                } else if let Some(d) = suffix.strip_prefix("sha512:") {
                    (Digest::SHA512(d), d, 128)
                } else {
                    return Err(ParseError::new(reference, "invalid digest algorithm"));
                }
            };

            if value.len() != expected_size || value.contains(|c: char| !c.is_ascii_hexdigit()) {
                return Err(ParseError::new(reference, "invalid digest"));
            }

            Ok((base, Some(digest)))
        }
    }
}

#[test]
fn parse_valid_references() {
    use crate::hex;
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

    let sha256 = hex::encode(Sha256::digest(b"\x00\x01"));
    let sha512 = hex::encode(Sha512::digest(b"\x01\x02"));

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
            Some(Digest::SHA256(&sha256))
        ]
    );

    check!(
        &format!("example.com:1234/foo/bar:1.2.3@sha512:{sha512}"),
        [
            "example.com:1234",
            Repository::Full("foo/bar"),
            "1.2.3",
            Some(Digest::SHA512(&sha512))
        ]
    );
}

#[test]
fn reject_invalid_digests() {
    assert!(matches!(
        Reference::parse("debian:stable@md5:0000"),
        Err(ParseError {
            message: "invalid digest algorithm",
            ..
        }),
    ));

    assert!(matches!(
        Reference::parse("debian:stable@sha256:0000"),
        Err(ParseError {
            message: "invalid digest",
            ..
        }),
    ));

    assert!(matches!(
        Reference::parse(&format!("debian:stable@sha256:{:064}", "x")),
        Err(ParseError {
            message: "invalid digest",
            ..
        }),
    ));
}
