//! Exact initialization-directory override of the packaged parent playbook.

use std::path::Path;

const NAME: &str = "PARENT_PLAYBOOK.md";

/// Return the project override if present, or the packaged default otherwise.
/// Existing but unreadable or invalid UTF-8 overrides fail rather than silently
/// changing the policy used for this session.
pub fn load(initialization_dir: &Path) -> Result<String, String> {
    let path = initialization_dir.join(NAME);
    let entries = std::fs::read_dir(initialization_dir).map_err(|error| {
        format!(
            "failed to inspect parent playbook at '{}': {error}",
            path.display()
        )
    })?;
    let mut present = false;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect parent playbook at '{}': {error}",
                path.display()
            )
        })?;
        if entry.file_name() == NAME {
            present = true;
            break;
        }
    }
    if !present {
        return Ok(include_str!("../../../PARENT_PLAYBOOK.md").to_owned());
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        format!(
            "failed to read parent playbook at '{}': {error}",
            path.display()
        )
    })?;
    String::from_utf8(bytes).map_err(|error| {
        format!(
            "parent playbook at '{}' is not valid UTF-8: {error}",
            path.display()
        )
    })
}
