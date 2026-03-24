use std::fs;
use std::path::PathBuf;

fn repo_file(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}

pub fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(repo_file(relative_path))
        .unwrap_or_else(|e| panic!("failed to read {relative_path}: {e}"))
}
