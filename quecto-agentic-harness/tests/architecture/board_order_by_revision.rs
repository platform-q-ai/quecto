//! The board orders changes by revision (epic #2705 B9): a task's
//! `updated` is a stamp for display, so no change is refused for a stamp
//! a burst of quick edits ran ahead of the clock. The burst bound that
//! did so is gone, by name, from the crate's source and from the agent's
//! playbook: neither its error nor its answer's code comes back.
use std::fs;
use std::path::{Path, PathBuf};

/// The names the burst bound went by: its error and its answer's code.
const RETIRED: &[&str] = &["TaskAhead", "task_ahead"];

/// The playbooks an agent reads, beside the crate's source.
const PLAYBOOKS: &[&str] = &["../PARENT_PLAYBOOK.md", "../COORDINATOR_PLAYBOOK.md"];

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        match (path.is_dir(), path.extension().and_then(|e| e.to_str())) {
            (true, _) => rust_files(&path, found),
            (false, Some("rs")) => found.push(path),
            (false, _) => {}
        }
    }
}

#[test]
fn the_burst_bound_is_gone_from_the_source_and_the_playbooks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(
        files.len() > 100,
        "the scan reads the crate: {} files",
        files.len()
    );
    files.extend(PLAYBOOKS.iter().map(|playbook| root.join(playbook)));
    let mut named = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for (number, line) in text.lines().enumerate() {
            if RETIRED.iter().any(|name| line.contains(name)) {
                named.push(format!("{}:{}: {line}", file.display(), number + 1));
            }
        }
    }
    assert!(
        named.is_empty(),
        "the burst bound is named:\n{}",
        named.join("\n")
    );
}
