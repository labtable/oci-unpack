use std::fmt::Write;

/// Encode `data` as hex string.
pub(crate) fn encode(data: impl AsRef<[u8]>) -> String {
    let data = data.as_ref();
    let mut output = String::with_capacity(data.len() * 2);

    for byte in data {
        let _ = write!(&mut output, "{:02X}", byte);
    }

    output
}

#[test]
fn encode_bytes() {
    assert_eq!(encode(b"\x01\x20\xf0"), "0120F0");
}
