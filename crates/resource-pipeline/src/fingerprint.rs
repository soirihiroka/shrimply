//! Content identity shared by headless resources and native media caches.
use sha2::{Digest, Sha256};
use std::io;
pub const HEX_DIGEST_BYTES: usize = 64;

/// Stream content into the shared hash without retaining a serialized copy.
#[derive(Default)]
pub struct Writer(Sha256);
impl Writer {
    pub fn finish(self) -> String {
        hex(self.0.finalize().as_ref())
    }
}
impl io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn digest_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub fn digest(bytes: &[u8]) -> String {
    hex(&digest_bytes(bytes))
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(HEX_DIGEST_BYTES);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing content fingerprint");
    }
    output
}
