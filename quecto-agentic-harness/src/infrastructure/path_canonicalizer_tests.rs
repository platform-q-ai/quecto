//! Integration tests: path canonicalization and symlinks (#2001 D2).

use super::*;
use crate::application::sessions::ports::PathCanonicalizer;
use std::fs;
use std::os::unix::fs::symlink;
use tempfile::TempDir;

#[test]
fn canonicalizes_existing_directory_to_absolute_path() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    fs::create_dir_all(&dir).unwrap();
    let c = FsPathCanonicalizer::new();
    let loc = c.canonicalize(dir.to_str().unwrap()).unwrap();
    assert!(loc.as_str().starts_with('/'));
    assert!(loc.as_str().ends_with("proj") || loc.as_str().contains("/proj"));
    // Round-trip: same directory yields same location text.
    let again = c.canonicalize(dir.to_str().unwrap()).unwrap();
    assert_eq!(loc, again);
}

#[test]
fn resolves_symlink_to_target_canonical_path() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real-dir");
    fs::create_dir_all(&real).unwrap();
    let link = tmp.path().join("link-dir");
    symlink(&real, &link).unwrap();
    let c = FsPathCanonicalizer::new();
    let loc = c.canonicalize(link.to_str().unwrap()).unwrap();
    let expected = fs::canonicalize(&real).unwrap();
    assert_eq!(loc.as_str(), expected.to_str().unwrap());
    assert_ne!(loc.as_str(), link.to_str().unwrap());
}

#[test]
fn missing_path_is_explicit_error_not_silent_fallback() {
    let c = FsPathCanonicalizer::new();
    let err = c
        .canonicalize("/definitely/missing/quecto-2001-path-xyz")
        .unwrap_err();
    assert!(
        err.to_string().contains("canonicalize") || err.to_string().contains("failed"),
        "{err}"
    );
}

#[test]
fn empty_path_is_refused() {
    let c = FsPathCanonicalizer::new();
    let err = c.canonicalize("").unwrap_err();
    assert!(err.to_string().contains("non-empty"), "{err}");
}
