//! Content identity shared by headless resources and native media caches.
use sha2::{Digest, Sha256};
pub fn digest_bytes(bytes: &[u8]) -> [u8; 32] { Sha256::digest(bytes).into() }
pub fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(64);
    for byte in digest_bytes(bytes) { write!(output, "{byte:02x}").expect("writing content fingerprint"); }
    output
}
