//! FNV-1a over length-prefixed fields: a digest that is stable across
//! processes and releases (unlike the standard hasher) and unambiguous across
//! field boundaries. Pure; the home version (#2011) is built on it.

pub(super) struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// One field: its length (u64, little-endian), then its bytes.
    pub(super) fn field(&mut self, bytes: &[u8]) {
        self.bytes(&(bytes.len() as u64).to_le_bytes());
        self.bytes(bytes);
    }

    pub(super) fn value(&self) -> u64 {
        self.0
    }
}
