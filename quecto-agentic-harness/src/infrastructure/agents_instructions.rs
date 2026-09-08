//! Initialization-directory `AGENTS.md` loading.
//!
//! This adapter intentionally performs one exact lookup. Ancestor traversal and
//! workspace/config-directory fallback do not belong to this policy.

use std::io::ErrorKind;
use std::path::Path;

const AGENTS_FILE_NAME: &str = "AGENTS.md";

/// Load `AGENTS.md` from exactly `initialization_dir`.
///
/// A missing file is not an error. Other read failures and malformed UTF-8 are
/// returned with the attempted path so the CLI can fail startup contextually.
pub fn load_agents_instructions(initialization_dir: &Path) -> Result<Option<String>, String> {
    let path = initialization_dir.join(AGENTS_FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read AGENTS.md at '{}': {error}",
                path.display()
            ));
        }
    };

    String::from_utf8(bytes).map(Some).map_err(|error| {
        format!(
            "AGENTS.md is not valid UTF-8 at '{}': {error}",
            path.display()
        )
    })
}
