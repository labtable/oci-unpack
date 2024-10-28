use oci_unpack::{MediaType, Reference, Unpacker};

pub mod common;

use common::{
    blobs::Blob,
    registry::{self, start_registry},
};

fn run_test(layers: Vec<Blob>) {
    let target = tempfile::tempdir().unwrap();

    let config_data = r#"{"test": true}"#;
    let config = Blob::new(MediaType::OciConfig, config_data.as_bytes());

    let port = start_registry("foo/bar", "0.1", config, layers);

    let reference = format!("127.0.0.1:{port}/foo/bar:0.1");
    let reference = Reference::try_from(reference.as_str());

    let result = Unpacker::new(reference.unwrap())
        .architecture(registry::ARCH)
        .os(registry::OS)
        .unpack(target.path());

    assert!(result.is_err())
}

#[test]
fn whiteout_parent() {
    run_test(vec![
        Blob::archive(false).regular("w/0/1", "w1").build(),
        Blob::archive(true).regular("w/0/.wh...", "").build(),
    ]);
}

#[test]
fn whiteout_current_dir() {
    run_test(vec![
        Blob::archive(false).regular("w/0/1", "w1").build(),
        Blob::archive(true).regular("w/0/.wh..", "").build(),
    ]);
}
