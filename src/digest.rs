use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Algorithm {
    SHA256,
    SHA512,
}

#[derive(Debug, PartialEq, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct Digest {
    hash: String,
    algorithm: Algorithm,
}

#[derive(thiserror::Error, Debug)]
pub enum DigestParseError {
    #[error("invalid digest algorithm")]
    InvalidDigestAlgorithm,

    #[error("invalid digest")]
    InvalidDigest,
}

impl Digest {
    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn hash_value(&self) -> &str {
        self.hash
            .split_once(':')
            .map(|(_, h)| h)
            .unwrap_or_default()
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }
}

impl TryFrom<String> for Digest {
    type Error = DigestParseError;

    fn try_from(hash: String) -> Result<Self, Self::Error> {
        let (algorithm, value, expected_size) = {
            if let Some(h) = hash.strip_prefix("sha256:") {
                (Algorithm::SHA256, h, 64)
            } else if let Some(h) = hash.strip_prefix("sha512:") {
                (Algorithm::SHA512, h, 128)
            } else {
                return Err(DigestParseError::InvalidDigestAlgorithm);
            }
        };

        if value.len() == expected_size && value.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Digest { hash, algorithm })
        } else {
            Err(DigestParseError::InvalidDigest)
        }
    }
}

/// Encode `data` as hex string.
pub(crate) fn hex_encode(data: impl AsRef<[u8]>) -> String {
    let data = data.as_ref();
    let mut output = String::with_capacity(data.len() * 2);

    for byte in data {
        let _ = write!(&mut output, "{:02X}", byte);
    }

    output
}

#[test]
fn encode_hex_bytes() {
    assert_eq!(hex_encode(b"\x01\x20\xf0"), "0120F0");
}
