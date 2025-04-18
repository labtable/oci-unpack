use std::{cell::RefCell, fmt, io::Write, path::Path, rc::Rc};

use flate2::write::GzEncoder;
use oci_unpack::MediaType;
use serde::ser::SerializeStruct;
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct Blob {
    pub media_type: MediaType,
    pub digest: String,
    pub data: Box<[u8]>,
}

impl serde::Serialize for Blob {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("Blob", 3)?;
        s.serialize_field("mediaType", self.media_type.as_str())?;
        s.serialize_field("digest", &format!("sha256:{}", self.digest))?;
        s.serialize_field("size", &self.data.len())?;
        s.end()
    }
}

impl Blob {
    pub fn new(media_type: MediaType, data: impl Into<Box<[u8]>>) -> Blob {
        let data = data.into();

        let mut hasher = Sha256::new();
        hasher.update(&data);
        let digest = HexString(hasher.finalize()).to_string();

        Blob {
            media_type,
            digest,
            data,
        }
    }

    /// Return a builder to create an archive.
    pub fn archive(media_type: MediaType) -> BlobArchive {
        let buffer = SharedBuffer(Rc::new(Vec::with_capacity(4096).into()));

        let stream: Box<dyn Write> = match media_type {
            MediaType::OciFsTarGzip => Box::new(GzEncoder::new(buffer.clone(), Default::default())),

            #[cfg(feature = "zstd")]
            MediaType::OciFsTarZstd => Box::new(
                zstd::stream::write::Encoder::new(buffer.clone(), 0)
                    .unwrap()
                    .auto_finish(),
            ),

            _ => Box::new(buffer.clone()),
        };

        BlobArchive {
            media_type,
            buffer,
            archive: tar::Builder::new(stream),
        }
    }
}

pub struct BlobArchive {
    media_type: MediaType,
    buffer: SharedBuffer,
    archive: tar::Builder<Box<dyn Write>>,
}

#[derive(Clone)]
struct SharedBuffer(Rc<RefCell<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl BlobArchive {
    pub fn build(mut self) -> Blob {
        self.archive.finish().unwrap();
        drop(self.archive.into_inner().unwrap());

        Blob::new(self.media_type, self.buffer.0.take())
    }

    pub fn directory(mut self, path: impl AsRef<Path>) -> Self {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::dir());
        header.set_size(0);
        header.set_cksum();
        self.archive.append(&header, &b""[..]).unwrap();
        self
    }

    pub fn regular(mut self, path: impl AsRef<Path>, data: impl AsRef<[u8]>) -> Self {
        let data = data.as_ref();
        let mut header = tar::Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::file());
        header.set_size(data.len() as u64);
        header.set_cksum();
        self.archive.append(&header, data).unwrap();
        self
    }

    pub fn symlink(mut self, path: impl AsRef<Path>, target: impl AsRef<Path>) -> Self {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::symlink());
        header.set_size(0);
        self.archive.append_link(&mut header, path, target).unwrap();
        self
    }
}

/// Encode a byte buffer as hex string.
struct HexString<T>(T);

impl<T: AsRef<[u8]>> fmt::Display for HexString<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .as_ref()
            .iter()
            .try_for_each(|byte| write!(f, "{:02x}", byte))
    }
}
