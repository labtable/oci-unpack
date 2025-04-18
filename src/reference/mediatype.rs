use std::{fmt, str::FromStr};

/// Generate the `MediaType` enum, its `FromStr` and `Display`
/// implementations, and the associated constant `ALL` with all
/// the valid values.
macro_rules! media_types {
    ($($variant:ident = $mediatype:expr,)*) => {
        /// Known media types.
        #[non_exhaustive]
        #[derive(Copy, Clone, PartialEq, Debug)]
        pub enum MediaType {
            $(
                #[doc = concat!("Variant for `", $mediatype, "`.")]
                $variant,
            )*
        }

        impl MediaType {
            /// List with all known media types.
            pub(crate) const ALL: &[&str] = &[ $($mediatype),* ];

            pub fn as_str(&self) -> &'static str {
                match self {
                    $(MediaType::$variant => $mediatype,)*
                }
            }
        }

        impl FromStr for MediaType {
            type Err = InvalidMediaType;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($mediatype => Ok(MediaType::$variant),)*
                    _ => Err(InvalidMediaType),
                }
            }
        }

        impl fmt::Display for MediaType {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    }
}

media_types!(
    DockerFsTarGzip = "application/vnd.docker.image.rootfs.diff.tar.gzip",
    DockerImageV1 = "application/vnd.docker.container.image.v1+json",
    DockerManifestList = "application/vnd.docker.distribution.manifest.list.v2+json",
    DockerManifestV2 = "application/vnd.docker.distribution.manifest.v2+json",
    OciConfig = "application/vnd.oci.image.config.v1+json",
    OciFsTar = "application/vnd.oci.image.layer.v1.tar",
    OciFsTarGzip = "application/vnd.oci.image.layer.v1.tar+gzip",
    OciFsTarZstd = "application/vnd.oci.image.layer.v1.tar+zstd",
    OciImageIndex = "application/vnd.oci.image.index.v1+json",
    OciManifestV1 = "application/vnd.oci.image.manifest.v1+json",
);

pub struct InvalidMediaType;

struct MediaTypeVisitor;

impl<'de> serde::de::Visitor<'de> for MediaTypeVisitor {
    type Value = MediaType;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("Media type for OCI/Docker objects.")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        MediaType::from_str(v).map_err(|_| E::custom(format!("Unknown type: {v}")))
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
            mt: MediaType::OciImageIndex
        })
    ));
}
