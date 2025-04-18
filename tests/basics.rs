use std::fs;

use oci_unpack::{MediaType, Reference, Unpacker};

mod common;

use common::{
    blobs::Blob,
    registry::{self, start_registry},
};

#[test]
fn multiple_layers_test() {
    let target = tempfile::tempdir().unwrap();

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

    let config_data = r#"{"test": true}"#;
    let config = Blob::new(MediaType::OciConfig, config_data.as_bytes());

    // Launch HTTP server for the registry and unpack the image.

    let port = start_registry("foo/bar", "0.1", config, layers);

    let reference = format!("127.0.0.1:{port}/foo/bar:0.1");
    let reference = Reference::try_from(reference.as_str());

    Unpacker::new(reference.unwrap())
        .architecture(registry::ARCH)
        .os(registry::OS)
        .unpack(target.path())
        .expect("Run unpacker");

    // Verify generated files.

    macro_rules! read {
        ($path:expr) => {
            fs::read(target.path().join($path)).expect($path)
        };
    }

    assert_eq!(read!("config.json"), config_data.as_bytes());

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
