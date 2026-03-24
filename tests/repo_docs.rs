use std::fs;
use std::path::PathBuf;

fn repo_file(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}

fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(repo_file(relative_path))
        .unwrap_or_else(|e| panic!("failed to read {relative_path}: {e}"))
}

#[test]
fn readme_license_section_matches_private_repo_status() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("## License"));
    assert!(readme.contains("LicenseRef-Proprietary"));
    assert!(readme.contains("private repository"));
    assert!(!readme.contains("## License\n\nMIT"));
}
