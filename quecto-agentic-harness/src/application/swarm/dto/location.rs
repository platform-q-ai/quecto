//! Where one board lives.
use std::path::PathBuf;

/// One board file: the SQLite `database` a run coordinates through, and
/// the `checkout` it belongs to (the working tree the members share).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardLocation {
    pub database: PathBuf,
    pub checkout: PathBuf,
}
