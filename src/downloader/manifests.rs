use std::{
    borrow::Cow,
    io::{BufReader, Read},
    str::FromStr,
};

use crate::{digest::Digest, EventHandler, Reference};

use super::{
    mime::{self, MediaType},
    DownloadError,
};

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct Blob {
    pub media_type: MediaType,
    pub digest: Digest,
}

#[derive(serde::Deserialize, Debug)]
pub(super) struct Manifest {
    pub config: Blob,
    pub layers: Vec<Blob>,
}

mod arch {
    #[cfg(target_arch = "aarch64")]
    pub(super) const DEFAULT: &str = "arm64";

    #[cfg(target_arch = "x86_64")]
    pub(super) const DEFAULT: &str = "amd64";

    #[cfg(target_arch = "riscv64")]
    pub(super) const DEFAULT: &str = "riscv64";
}

#[cfg(windows)]
const DEFAULT_OS: &str = "windows";

#[cfg(not(windows))]
const DEFAULT_OS: &str = "linux";

pub(super) fn get<E: EventHandler>(
    reference: &Reference,
    architecture: Option<&str>,
    os: Option<&str>,
    http_client: &mut super::http::Client<E>,
) -> Result<Manifest, DownloadError> {
    let architecture = architecture.unwrap_or(arch::DEFAULT);
    let os = os.unwrap_or(DEFAULT_OS);

    enum Tag<'a> {
        S(&'a str),
        D(Cow<'a, Digest>),
    }

    let accept = mime::MediaType::ALL.join(", ");

    let mut tag = match reference.digest.as_ref() {
        Some(d) => Tag::D(Cow::Borrowed(d)),
        None => Tag::S(reference.tag),
    };

    loop {
        let path = match &tag {
            Tag::S(s) => s,
            Tag::D(s) => s.hash(),
        };

        let response = http_client.get(
            &format!("v2/{}/manifests/{}", &reference.repository, path),
            Some(&accept),
        )?;

        let content_type = response
            .header("Content-Type")
            .and_then(|h| MediaType::from_str(h).ok())
            .ok_or(DownloadError::InvalidContentType)?;

        // If we have an expected digest, compute it during the download,
        // and verify it when the download is completed.
        let mut body: Box<dyn Read> = {
            let response = response.into_reader();
            match &tag {
                Tag::D(d) => Box::new(BufReader::new(d.wrap_reader(response))),
                Tag::S(_) => Box::new(response),
            }
        };

        tag = match content_type {
            MediaType::DockerManifestList | MediaType::OciManifestIndex => {
                Tag::D(Cow::Owned(parse_index(architecture, os, &mut body)?))
            }

            MediaType::DockerManifestV2 | MediaType::OciManifestV1 => {
                // https://distribution.github.io/distribution/spec/manifest-v2-2/
                return Ok(serde_json::from_reader(&mut body)?);
            }

            _ => {
                return Err(DownloadError::InvalidContentType);
            }
        }
    }
}

/// Parse a manifest/index to get the digest for the specified architecture and
/// operating system.
///
/// Refs:
///
/// * https://distribution.github.io/distribution/spec/manifest-v2-2/#manifest-list
/// * https://github.com/opencontainers/image-spec/blob/main/image-index.md
fn parse_index(
    architecture: &str,
    os: &str,
    response: &mut dyn Read,
) -> Result<Digest, DownloadError> {
    #[derive(serde::Deserialize, Debug)]
    struct List {
        manifests: Vec<Item>,
    }

    #[derive(serde::Deserialize, Debug)]
    struct Item {
        digest: String,
        platform: Platform,
    }

    #[derive(serde::Deserialize, Debug)]
    struct Platform {
        architecture: String,
        os: String,
    }

    let List { manifests } = dbg!(serde_json::from_reader(response)?);
    let item = manifests
        .into_iter()
        .find(|i| i.platform.architecture == architecture && i.platform.os == os)
        .ok_or(DownloadError::MissingArchitecture)?;

    Ok(Digest::try_from(item.digest)?)
}
