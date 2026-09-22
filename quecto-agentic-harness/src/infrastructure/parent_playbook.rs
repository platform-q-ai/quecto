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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_packaged_playbook_only_when_absent() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            load(directory.path()).unwrap(),
            include_str!("../../../PARENT_PLAYBOOK.md")
        );
        std::fs::write(directory.path().join(NAME), "project policy\n").unwrap();
        assert_eq!(load(directory.path()).unwrap(), "project policy\n");
    }

    #[test]
    fn rejects_invalid_and_unreadable_override() {
        let directory = tempfile::tempdir().unwrap();
        let override_path = directory.path().join(NAME);
        std::fs::write(&override_path, [0xff]).unwrap();
        assert!(load(directory.path()).unwrap_err().contains("UTF-8"));
        std::fs::remove_file(&override_path).unwrap();
        std::fs::create_dir(&override_path).unwrap();
        assert!(
            load(directory.path())
                .unwrap_err()
                .contains("failed to read")
        );
    }

    #[test]
    fn never_searches_ancestors() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(NAME), "ancestor").unwrap();
        let child = directory.path().join("child");
        std::fs::create_dir(&child).unwrap();
        assert_eq!(
            load(&child).unwrap(),
            include_str!("../../../PARENT_PLAYBOOK.md")
        );
    }
}
