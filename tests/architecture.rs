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

/// Check that a specific source file does not contain forbidden patterns in
/// production code (everything after `#[cfg(test)]` is skipped).
fn assert_file_no_patterns(file: &str, forbidden: &[&str], rule: &str) {
    let content = fs::read_to_string(file).expect("read file");

    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Stop scanning this file once we hit a test module.
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
                    "Architecture violation: {file}\n\
                     Line {}: {trimmed}\n\
                     Forbidden pattern: {pattern}\n\
                     Rule: {rule}",
                    line_no + 1
                );
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
fn application_layer_has_no_runtime_io_calls() {
    assert_no_imports(
        "application",
        Path::new("src/application"),
        &["std::fs::", "tokio::fs::", "std::env::", "dirs::"],
    );

    // Extra guard: filesystem existence checks are also I/O and should move to infrastructure ports.
    assert_no_imports("application", Path::new("src/application"), &[".exists("]);
}

#[test]
fn domain_channel_trait_is_dyn_compatible() {
    let file = "src/domain/channel.rs";
    let content = fs::read_to_string(file).expect("read domain/channel.rs");

    assert!(
        content.contains("Pin<Box<dyn Future"),
        "Architecture violation: {file}\n\
         Rule: domain ports must be dyn-compatible and use boxed futures (Pin<Box<dyn Future + Send + '_>>)",
    );

    assert!(
        !content.contains("-> impl std::future::Future"),
        "Architecture violation: {file}\n\
         Rule: domain ports must not use RPITIT return types (-> impl Future) because they break dyn trait compatibility",
    );
}

#[test]
fn gateway_runtime_context_uses_domain_ports_not_concrete_types() {
    let file = "src/interface/gateway/mod.rs";
    let content = fs::read_to_string(file).expect("read gateway/mod.rs");

    for pattern in [
        "pub(super) agent: Arc<AgentLoopImpl>",
        "pub(super) session_store: Arc<FileSessionStore>",
        "pub(super) telegram: TelegramChannel",
    ] {
        assert!(
            !content.contains(pattern),
            "Architecture violation: {file}\n\
             Forbidden pattern: {pattern}\n\
             Rule: gateway event-loop context should depend on domain ports (Arc<dyn AgentLoop>, Arc<dyn SessionStore>, Arc<dyn Channel>)",
        );
    }
}

#[test]
fn gateway_services_signatures_use_ports_not_concrete_types() {
    assert_file_no_patterns(
        "src/interface/gateway/services.rs",
        &[
            "agent: Arc<AgentLoopImpl>",
            "agent: &Arc<AgentLoopImpl>",
            "session_store: Arc<FileSessionStore>",
            "session_store: &Arc<FileSessionStore>",
            "telegram: TelegramChannel",
        ],
        "gateway services should accept trait ports instead of concrete infrastructure/application types",
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
