use super::write_atomic;
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn write_atomic_creates_parent_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/registry.json");
    write_atomic(&path, br#"{"ok":true}"#).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"ok":true}"#);
}

#[test]
fn write_atomic_replaces_existing_without_leaving_tmp() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    write_atomic(&path, b"v1").unwrap();
    write_atomic(&path, b"v2").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "v2");
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "tmp leftovers: {leftovers:?}");
}

#[test]
fn concurrent_readers_never_see_empty_partial_after_replace() {
    // Best-effort: after each successful write, file content is one of the
    // complete payloads (never empty/truncated mid-write at the destination).
    let dir = tempfile::tempdir().unwrap();
    let path = Arc::new(dir.path().join("sidecar.json"));
    write_atomic(&path, br#"{"n":0}"#).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let writer = {
        let path = Arc::clone(&path);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            for n in 1..=50 {
                let body = format!(r#"{{"n":{n}}}"#);
                write_atomic(&path, body.as_bytes()).unwrap();
            }
        })
    };
    let reader = {
        let path = Arc::clone(&path);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            for _ in 0..200 {
                if let Ok(s) = fs::read_to_string(&*path) {
                    assert!(s.starts_with('{') && s.ends_with('}'), "partial: {s:?}");
                    assert!(s.contains("\"n\":"), "partial: {s:?}");
                }
            }
        })
    };
    writer.join().unwrap();
    reader.join().unwrap();
}
