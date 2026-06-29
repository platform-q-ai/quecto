mod common;

use common::read_repository_file;

#[test]
fn read_repository_file_reads_within_repo_root() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let content = read_repository_file(base, "README.md").expect("should read README.md");
    assert!(content.contains("# Quecto"));
}

#[test]
fn read_repository_file_rejects_path_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    let outside = temp.path().join("outside.txt");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    std::fs::write(repo_root.join("inside.txt"), "inside").expect("write inside file");
    std::fs::write(&outside, "outside").expect("write outside file");

    let err = read_repository_file(&repo_root, "../outside.txt").expect_err("should reject escape");
    assert!(
        err.contains("path escapes repo root"),
        "unexpected error: {}",
        err
    );
}
