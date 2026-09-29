//! The shared checkout's paths (#2275): `CheckoutPaths` over the
//! filesystem, with Python's non-strict `Path.resolve()`.
use std::path::PathBuf;

use crate::application::swarm::ports::CheckoutPaths;
use crate::domain::swarm::BoardError;

/// The checkout at `root`.
#[derive(Clone, Debug)]
pub struct ResolvedCheckout {
    root: PathBuf,
}

impl ResolvedCheckout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl CheckoutPaths for ResolvedCheckout {
    fn normalize(&self, path: &str) -> Result<String, BoardError> {
        let _ = (&self.root, path);
        Err(BoardError::new("not served yet"))
    }
}

#[cfg(test)]
#[path = "checkout_paths_tests.rs"]
mod tests;
