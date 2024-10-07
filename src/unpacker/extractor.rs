use std::{
    cell::Cell,
    fs::File,
    io::{self, BufReader, Read, Seek},
};

use crate::{fs::Directory, EventHandler, MediaType};

use super::manifests::Blob;

pub(crate) fn extract<E: EventHandler>(
    event_handler: &E,
    _target: &Directory,
    blob: &Blob,
    mut tarball: File,
) -> io::Result<()> {
    let archive_len = tarball.seek(io::SeekFrom::End(0))?;
    tarball.rewind()?;

    let tarball_position = Cell::new(0);
    let tarball = PositionTracker {
        count: &tarball_position,
        reader: tarball,
    };

    let reader: Box<dyn Read> = match blob.media_type {
        MediaType::DockerImageV1 | MediaType::OciConfig => {
            // Configuration files are just written to disk.
            return Ok(());
        }

        MediaType::DockerFsTarGzip | MediaType::OciFsTarGzip => {
            Box::new(flate2::read::GzDecoder::new(tarball))
        }

        MediaType::OciFsTar => Box::new(BufReader::new(tarball)),

        unknown => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid media type: {unknown}"),
            ))
        }
    };

    event_handler.layer_start(archive_len);

    let mut archive = tar::Archive::new(reader);

    for entry in archive.entries()? {
        event_handler.layer_progress(tarball_position.get());
        let _entry = entry?;

        // TODO write files ; apply whiteouts/opaque marks
    }

    event_handler.layer_progress(tarball_position.get());

    Ok(())
}

/// Count how many bytes have been read from `reader`.
struct PositionTracker<'a, R> {
    count: &'a Cell<usize>,
    reader: R,
}

impl<T: Read> Read for PositionTracker<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        self.count.set(self.count.get() + n);
        Ok(n)
    }
}
