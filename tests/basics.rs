use std::{assert_matches, fs, io::ErrorKind::DirectoryNotEmpty};

use oci_unpack::{errors::UnpackError, MediaType, Reference, Unpacker};

mod common;

use common::{
    blobs::Blob,
    registry::{self, start_registry},
};

const CONFIG_DATA: &str = r#"{"test": true}"#;

struct ExampleImage {
    reference: String,
}

impl ExampleImage {
    fn new() -> ExampleImage {
        let layers = vec![
            // Layer with regular files.
            Blob::archive(MediaType::OciFsTarGzip)
                .directory("abc")
                .regular("abc/def", "a1")
                .regular("./0/1", "a2")
                .build(),
            //
            // Layer trying to escape from rootfs;
            Blob::archive(MediaType::OciFsTar)
                .directory("dot0/dot1")
                .symlink("dot0/dot2", "../../../../../dot0/dot1")
                .regular("./dot0/dot2/file", "b1")
                .build(),
            //
            // Whiteouts. First, create some files in a layer, that will
            // be removed in the next one.
            Blob::archive(MediaType::OciFsTar)
                .regular("w/0/1", "w1")
                .regular("w/0/2", "w2")
                .regular("w/1/3", "w3")
                .regular("w/1/4", "w4")
                .regular("w/2/5", "w5")
                .build(),
            Blob::archive(MediaType::OciFsTarGzip)
                .regular("w/0/.wh.1", "")
                .regular("w/0/.wh.must-be-ignored", "")
                .regular("w/1/.wh..wh..opq", "")
                .regular("w/.wh.2", "")
                .build(),
            //
            // A layer compressed with zstd.
            #[cfg(feature = "zstd")]
            Blob::archive(MediaType::OciFsTarZstd)
                .regular("from.zstd", "01234")
                .build(),
        ];

        let config = Blob::new(MediaType::OciConfig, CONFIG_DATA.as_bytes());

        // Launch HTTP server for the registry and unpack the image.

        let port = start_registry("foo/bar", "0.1", config, layers);
        let reference = format!("127.0.0.1:{port}/foo/bar:0.1");

        ExampleImage { reference }
    }

    fn reference<'a>(&'a self) -> Reference<'a> {
        Reference::try_from(self.reference.as_str()).unwrap()
    }
}

fn run_unpack(target: &tempfile::TempDir) -> Result<(), UnpackError> {
    let example = ExampleImage::new();

    Unpacker::new(example.reference())
        .architecture(registry::ARCH)
        .os(registry::OS)
        .unpack(target.path())
}

#[test]
fn multiple_layers_test() {
    let target = tempfile::tempdir().unwrap();

    run_unpack(&target).expect("Run unpacker");

    // Verify generated files.

    macro_rules! read {
        ($path:expr) => {
            fs::read(target.path().join($path)).expect($path)
        };
    }

    assert_eq!(read!("config.json"), CONFIG_DATA.as_bytes());

    assert_eq!(read!("rootfs/abc/def"), b"a1");
    assert_eq!(read!("rootfs/0/1"), b"a2");
    assert_eq!(read!("rootfs/dot0/dot1/file"), b"b1");
    assert_eq!(read!("rootfs/w/0/2"), b"w2");

    // Paths that must be removed.
    for path in ["rootfs/w/0/1", "rootfs/w/2"] {
        assert!(!target.path().join(path).exists());
    }

    // `w/1` should be empty.
    assert!(fs::read_dir(target.path().join("rootfs/w/1"))
        .unwrap()
        .next()
        .is_none());

    // Files from a zstd layer
    #[cfg(feature = "zstd")]
    assert_eq!(read!("rootfs/from.zstd"), b"01234");
}

#[test]
fn accept_empty_rootfs() {
    let target = tempfile::tempdir().unwrap();
    fs::create_dir_all(target.path().join("rootfs")).unwrap();
    assert_matches!(run_unpack(&target), Ok(_));
}

#[test]
fn reject_non_empty_target() {
    let target = tempfile::tempdir().unwrap();

    fs::create_dir_all(target.path().join("not-rootfs")).unwrap();

    assert_matches!(run_unpack(&target), Err(UnpackError::Io(e, _)) if e.kind() == DirectoryNotEmpty);
}

#[test]
fn reject_rootfs_with_others() {
    let target = tempfile::tempdir().unwrap();

    fs::create_dir_all(target.path().join("not-rootfs")).unwrap();
    fs::create_dir_all(target.path().join("rootfs")).unwrap();

    assert_matches!(run_unpack(&target), Err(UnpackError::Io(e, _)) if e.kind() == DirectoryNotEmpty);
}

#[test]
fn reject_non_empty_rootfs_target() {
    let target = tempfile::tempdir().unwrap();

    let rootfs = target.path().join("rootfs");
    fs::create_dir_all(&rootfs).unwrap();
    fs::write(rootfs.join("something"), "").unwrap();

    assert_matches!(run_unpack(&target), Err(UnpackError::Io(e, _)) if e.kind() == DirectoryNotEmpty);
}
