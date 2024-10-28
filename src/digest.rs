use std::{
    fmt,
    io::{self, Read},
};

use sha2::Digest as _;

/// Algorithm to compute the hash value.
///
/// See [`Digest`] for an example.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    SHA256,
    SHA512,
}

/// A digest to validate a blob.
///
/// It contains the algorithm (like `SHA256`) and its expected value as
/// a hexadecimal string.
///
/// # Examples
///
/// ```
/// # use oci_unpack::*;
/// const DIGEST: &str = "123456789012345678901234567890123456789012345678901234567890ABCD";
///
/// let digest = Digest::try_from(format!("sha256:{}", DIGEST)).unwrap();
/// assert_eq!(digest.algorithm(), DigestAlgorithm::SHA256);
/// assert_eq!(digest.hash_value(), DIGEST);
/// ```
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct Digest {
    hash: String,
    algorithm: DigestAlgorithm,
}

/// Errors from the digest parser.
#[derive(thiserror::Error, Debug)]
pub enum DigestError {
    #[error("Invalid digest algorithm.")]
    InvalidAlgorithm,

    #[error("Invalid digest value.")]
    InvalidValue,
}

impl Digest {
    /// Original string to build this instance (`algorithm:hash_value`).
    pub fn source(&self) -> &str {
        &self.hash
    }

    pub fn hash_value(&self) -> &str {
        self.hash
            .split_once(':')
            .map(|(_, h)| h)
            .unwrap_or_default()
    }

    pub fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    /// Return a `Read` instance to compute its digest.
    ///
    /// When all data from `reader` is consumed, it verifies that the
    /// computed digest is the expected one. If not, it returns an
    /// [`InvalidData`](::std::io::ErrorKind::InvalidData)
    /// error.
    pub fn wrap_reader<R: Read>(&self, reader: R) -> impl Read {
        let hasher: Box<dyn digest::DynDigest> = match self.algorithm {
            DigestAlgorithm::SHA256 => Box::new(sha2::Sha256::new()),
            DigestAlgorithm::SHA512 => Box::new(sha2::Sha512::new()),
        };

        DigestReader {
            hasher,
            expected: self.hash_value().to_owned(),
            reader,
        }
    }
}

impl TryFrom<String> for Digest {
    type Error = DigestError;

    fn try_from(hash: String) -> Result<Self, Self::Error> {
        let (algorithm, value, expected_size) = {
            if let Some(h) = hash.strip_prefix("sha256:") {
                (DigestAlgorithm::SHA256, h, 256 / 8 * 2)
            } else if let Some(h) = hash.strip_prefix("sha512:") {
                (DigestAlgorithm::SHA512, h, 512 / 8 * 2)
            } else {
                return Err(DigestError::InvalidAlgorithm);
            }
        };

        // Validate that the hash value is a string with the expected length,
        // and it only contains hexadecimal digits.
        if value.len() == expected_size && value.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Digest { hash, algorithm })
        } else {
            Err(DigestError::InvalidValue)
        }
    }
}

struct DigestReader<R> {
    hasher: Box<dyn digest::DynDigest>,
    expected: String,
    reader: R,
}

impl<R: Read> Read for DigestReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let buf_len = buf.len();
        let n = self.reader.read(buf)?;

        if n == 0 && buf_len > 0 {
            // On EOF, compare the computed digest with the expected one.
            return self.check_hash();
        }

        self.hasher.update(&buf[..n]);

        Ok(n)
    }
}

impl<R> DigestReader<R> {
    fn check_hash(&mut self) -> io::Result<usize> {
        const MAX_DIGEST_SIZE: usize = 512 / 8;

        debug_assert_eq!(self.hasher.output_size() * 2, self.expected.len());

        let mut buffer = [0u8; MAX_DIGEST_SIZE];
        let out = &mut buffer[..self.hasher.output_size()];

        self.hasher
            .finalize_into_reset(out)
            .map_err(io::Error::other)?;

        let mut expected = self.expected.as_str();
        for hash_byte in out.iter() {
            match expected
                .split_at_checked(2)
                .map(|(b, t)| (u8::from_str_radix(b, 16), t))
            {
                Some((Ok(byte), t)) if byte == *hash_byte => expected = t,

                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Invalid digest. Expected {}, got {}.",
                            self.expected,
                            HexString(out)
                        ),
                    ))
                }
            }
        }

        Ok(0)
    }
}

/// Encode a byte buffer as hex string.
pub(crate) struct HexString<T>(pub T);

impl<T: AsRef<[u8]>> fmt::Display for HexString<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .as_ref()
            .iter()
            .try_for_each(|byte| write!(f, "{:02x}", byte))
    }
}

#[test]
fn encode_hex_bytes() {
    assert_eq!(HexString(b"\x01\x20\xf0").to_string(), "0120f0");
}

#[test]
fn reject_invalid_digest() {
    use std::io::Cursor;

    /// Digest for `abc`
    const DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    let digest = Digest::try_from(format!("sha256:{DIGEST}")).unwrap();
    let mut output = Vec::new();

    // Accept a valid digest.
    digest
        .wrap_reader(Cursor::new("abc"))
        .read_to_end(&mut output)
        .unwrap();

    assert_eq!(output, b"abc");

    // Reject an invalid digest.
    output.clear();
    let err = digest
        .wrap_reader(Cursor::new("abcx"))
        .read_to_end(&mut output)
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    let msg = err.into_inner().unwrap().to_string().to_lowercase();
    assert!(msg.contains(DIGEST));
    assert!(msg.contains("7571ce1f8e21c6b13dd7ec2c5ec7c9e4dd9852e209869511853f2f1f74b17927"));
}
