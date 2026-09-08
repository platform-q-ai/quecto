//! Operating-system randomness for capability material.
use crate::application::ports::AdmissionSecretSource;
use rand::RngCore;

/// 256 bits of OS randomness rendered as lowercase hex (64 alphanumeric chars).
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomSecretSource;

impl AdmissionSecretSource for RandomSecretSource {
    fn mint(&mut self) -> String {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let mut out = String::with_capacity(64);
        for byte in bytes {
            use std::fmt::Write;
            write!(out, "{byte:02x}").expect("hex into String cannot fail");
        }
        out
    }
}

#[cfg(test)]
#[path = "secret_tests.rs"]
mod tests;
