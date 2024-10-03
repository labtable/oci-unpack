use std::{fmt, str::FromStr};

/// Generate the `MediaType` enum, its `FromStr` implementation, and
/// the associated constant `ALL` with all the valid MIME types.
macro_rules! mime_strings {
    ($($variant:ident = $mime:expr,)*) => {
        #[derive(PartialEq, Debug)]
        pub(super) enum MediaType {
            $($variant),*
        }

        impl MediaType {
            pub const ALL: &[&str] = &[ $($mime),* ];
        }

        impl FromStr for MediaType {
            type Err = InvalidMediaType;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($mime => Ok(MediaType::$variant),)*
                    _ => Err(InvalidMediaType),
                }
            }
        }
    }
}

mime_strings!(
    DockerFsTarGzip = "application/vnd.docker.image.rootfs.diff.tar.gzip",
    DockerImageV1 = "application/vnd.docker.container.image.v1+json",
    DockerManifestList = "application/vnd.docker.distribution.manifest.list.v2+json",
    DockerManifestV2 = "application/vnd.docker.distribution.manifest.v2+json",
    OciConfig = "application/vnd.oci.image.config.v1+json",
    OciFsTarGzip = "application/vnd.oci.image.layer.v1.tar+gzip",
    OciManifestIndex = "application/vnd.oci.image.index.v1+json",
    OciManifestV1 = "application/vnd.oci.image.manifest.v1+json",
);

pub(super) struct InvalidMediaType;

struct MediaTypeVisitor;

impl<'de> serde::de::Visitor<'de> for MediaTypeVisitor {
    type Value = MediaType;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("Image/manifest MIME type.")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        MediaType::from_str(v).map_err(|_| E::custom(format!("Unknown MIME: {v}")))
    }
}

impl<'de> serde::Deserialize<'de> for MediaType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(MediaTypeVisitor)
    }
}

#[test]
fn media_type_in_json() {
    #[derive(serde::Deserialize, Debug)]
    struct Example {
        mt: MediaType,
    }

    assert!(matches!(
        serde_json::from_str(r#"{"mt": "application/vnd.oci.image.index.v1+json"}"#),
        Ok(Example {
            mt: MediaType::OciManifestIndex
        })
    ));
}
