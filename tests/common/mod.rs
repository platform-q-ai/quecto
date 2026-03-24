#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

fn repo_file(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}

pub fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(repo_file(relative_path))
        .unwrap_or_else(|e| panic!("failed to read {relative_path}: {e}"))
}

pub fn read_repository_file(base: &Path, relative_path: &str) -> Result<String, String> {
    let base = base
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize repo root {:?}: {}", base, e))?;
    let path = base.join(relative_path);
    let resolved = path
        .canonicalize()
        .map_err(|e| format!("failed to resolve {:?}: {}", path, e))?;
    if !resolved.starts_with(&base) {
        return Err(format!("path escapes repo root: {:?}", resolved));
    }
    fs::read_to_string(&resolved).map_err(|e| format!("failed to read {:?}: {}", resolved, e))
}
