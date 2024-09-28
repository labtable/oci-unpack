use std::fmt::Write;

#[derive(Debug, PartialEq)]
pub enum Digest<'a> {
    SHA256(&'a str),
    SHA512(&'a str),
}

#[derive(thiserror::Error, Debug)]
pub enum DigestParseError {
    #[error("invalid digest algorithm")]
    InvalidDigestAlgorithm,

    #[error("invalid digest")]
    InvalidDigest,
}

impl<'a> Digest<'a> {
    pub fn parse(digest: &'a str) -> Result<Self, DigestParseError> {
        let (digest, value, expected_size) = {
            if let Some(d) = digest.strip_prefix("sha256:") {
                (Digest::SHA256(d), d, 64)
            } else if let Some(d) = digest.strip_prefix("sha512:") {
                (Digest::SHA512(d), d, 128)
            } else {
                return Err(DigestParseError::InvalidDigestAlgorithm);
            }
        };

        if value.len() == expected_size && value.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(digest)
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
