use std::{
    borrow::Cow,
    env::consts,
    io::{BufReader, Read},
    str::FromStr,
};

use crate::{digest::Digest, unpacker::UnpackError, EventHandler, MediaType, Reference};

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(super) struct Blob {
    pub media_type: MediaType,
    pub digest: Digest,
    pub size: usize,
}

#[derive(serde::Deserialize, Debug)]
pub(super) struct Manifest {
    pub config: Blob,
    pub layers: Vec<Blob>,
}

/// Download the manifest for the `reference`.
///
/// It tries to download a manifest index to locate the image
/// for the `architecture`/`os` pair.
///
/// If `reference` contains a digest (like `@sha256:...`), the
/// manifest index is skipped.
pub(super) fn get<E: EventHandler>(
    reference: &Reference,
    architecture: Option<&str>,
    os: Option<&str>,
    http_client: &mut crate::http::Client<E>,
) -> Result<Manifest, UnpackError> {
    // Translate to golang architecture names.
    let default_arch = match consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "386",
        "x86_64" => "amd64",
        other => other,
    };

    let architecture = architecture.unwrap_or(default_arch);
    let os = os.unwrap_or(consts::OS);

    enum Tag<'a> {
        S(&'a str),
        D(Cow<'a, Digest>),
    }

    let accept = MediaType::ALL.join(", ");

    let mut tag = match reference.digest.as_ref() {
        Some(d) => Tag::D(Cow::Borrowed(d)),
        None => Tag::S(reference.tag),
    };

    loop {
        let path = match &tag {
            Tag::S(s) => s,
            Tag::D(s) => s.source(),
        };

        let response = http_client.get(&format!("manifests/{}", path), Some(&accept))?;

        let content_type = response
            .header("Content-Type")
            .and_then(|h| MediaType::from_str(h).ok())
            .ok_or(UnpackError::MissingContentType)?;

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
            MediaType::DockerManifestList | MediaType::OciImageIndex => {
                Tag::D(Cow::Owned(parse_index(architecture, os, &mut body)?))
            }

            MediaType::DockerManifestV2 | MediaType::OciManifestV1 => {
                // https://distribution.github.io/distribution/spec/manifest-v2-2/
                return Ok(serde_json::from_reader(&mut body)?);
            }

            unknown => {
                return Err(UnpackError::InvalidContentType(unknown));
            }
        }
    }
}

/// Parse an image index to get the digest for the specified architecture and
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
) -> Result<Digest, UnpackError> {
    #[derive(serde::Deserialize, Debug)]
    struct List {
        manifests: Vec<Manifest>,
    }

    #[derive(serde::Deserialize, Debug)]
    struct Manifest {
        digest: String,
        platform: Platform,
    }

    #[derive(serde::Deserialize, Debug)]
    struct Platform {
        architecture: String,
        os: String,
    }

    let List { manifests } = serde_json::from_reader(response)?;
    let item = manifests
        .into_iter()
        .find(|i| i.platform.architecture == architecture && i.platform.os == os)
        .ok_or(UnpackError::MissingArchitecture)?;

    Ok(Digest::try_from(item.digest)?)
}
