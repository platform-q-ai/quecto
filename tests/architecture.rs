//! Architecture boundary enforcement tests.
//!
//! These tests scan source files to ensure Clean Architecture dependency rules
//! are respected. They run on every commit (pre-commit hook) and every push.
//!
//! Dependency rule:
//!   interface/ --> application/ --> domain/
//!                      |
//!   infrastructure/ ---+
//!
//! - domain/ imports nothing from the project
//! - application/ imports domain/ only
//! - infrastructure/ imports domain/ only
//! - interface/ imports all three (composition root)

use std::fs;
use std::path::Path;

/// Recursively collect all .rs files under a directory.
fn collect_rs_files(dir: &Path, files: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(&path).expect("read file");
            files.push(format!("{}:\n{}", path.display(), content));
        }
    }
}

/// Check that no file in `dir` contains any of the `forbidden` import patterns.
/// Only checks production code — everything after `#[cfg(test)]` is skipped.
fn assert_no_imports(layer: &str, dir: &Path, forbidden: &[&str]) {
    let mut files = Vec::new();
    collect_rs_files(dir, &mut files);

    for file_content in &files {
        let (file_path, _) = file_content.split_once(":\n").unwrap();

        for line in file_content.lines().skip(1) {
            let trimmed = line.trim();

            // Stop scanning this file once we hit a test module.
            // Convention: #[cfg(test)] appears at the end of the file.
            if trimmed == "#[cfg(test)]" {
                break;
            }

            // Skip comments
            if trimmed.starts_with("//") {
                continue;
            }
            for pattern in forbidden {
                if trimmed.contains(pattern) {
                    panic!(
                        "Architecture violation in {layer}: {file_path}\n\
                         Line: {trimmed}\n\
                         Forbidden import: {pattern}\n\
                         Rule: {layer}/ must not import from {}",
                        pattern
                            .trim_start_matches("crate::")
                            .split("::")
                            .nth(1)
                            .unwrap_or(pattern)
                    );
                }
            }
        }
    }
}

#[test]
fn all_architecture_layers_exist() {
    assert!(Path::new("src/domain").exists(), "src/domain/ must exist");
    assert!(
        Path::new("src/application").exists(),
        "src/application/ must exist"
    );
    assert!(
        Path::new("src/infrastructure").exists(),
        "src/infrastructure/ must exist"
    );
    assert!(
        Path::new("src/interface").exists(),
        "src/interface/ must exist"
    );
}

#[test]
fn domain_has_no_application_imports() {
    assert_no_imports("domain", Path::new("src/domain"), &["crate::application"]);
}

#[test]
fn domain_has_no_infrastructure_imports() {
    assert_no_imports(
        "domain",
        Path::new("src/domain"),
        &["crate::infrastructure"],
    );
}

#[test]
fn domain_has_no_interface_imports() {
    assert_no_imports("domain", Path::new("src/domain"), &["crate::interface"]);
}

#[test]
fn application_has_no_infrastructure_imports() {
    assert_no_imports(
        "application",
        Path::new("src/application"),
        &["crate::infrastructure"],
    );
}

#[test]
fn application_has_no_interface_imports() {
    assert_no_imports(
        "application",
        Path::new("src/application"),
        &["crate::interface"],
    );
}

#[test]
fn infrastructure_has_no_application_imports() {
    assert_no_imports(
        "infrastructure",
        Path::new("src/infrastructure"),
        &["crate::application"],
    );
}

#[test]
fn infrastructure_has_no_interface_imports() {
    assert_no_imports(
        "infrastructure",
        Path::new("src/infrastructure"),
        &["crate::interface"],
    );
}

#[test]
fn lib_rs_has_deny_attributes() {
    let content = fs::read_to_string("src/lib.rs").expect("read lib.rs");
    assert!(
        content.contains("#![deny(dead_code)]"),
        "src/lib.rs must contain #![deny(dead_code)]"
    );
    assert!(
        content.contains("#![deny(unused_imports)]"),
        "src/lib.rs must contain #![deny(unused_imports)]"
    );
}
