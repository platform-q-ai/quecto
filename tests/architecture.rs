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

use std::collections::BTreeSet;
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
            // Files named `*_tests.rs` are test-only modules included via
            // `#[cfg(test)]`, not production architecture dependencies.
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                continue;
            }
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
fn domain_layer_has_no_runtime_io_calls() {
    assert_no_imports(
        "domain",
        Path::new("src/domain"),
        &[
            "std::fs::",
            "tokio::fs::",
            "std::env::",
            "dirs::",
            ".exists(",
        ],
    );
}

#[test]
fn public_ports_have_contract_tests() {
    let mut files = Vec::new();
    collect_rs_files(Path::new("src/domain"), &mut files);
    collect_rs_files(Path::new("src/application"), &mut files);

    let mut ports = BTreeSet::new();
    for file_content in &files {
        for line in file_content.lines().skip(1) {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("pub trait ") {
                if let Some(name) = rest
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .find(|part| !part.is_empty())
                {
                    ports.insert(name.to_string());
                }
            }
        }
    }

    let contract_modules = active_contract_modules();
    let allowlisted: BTreeSet<&str> = BTreeSet::new();
    let missing: Vec<_> = ports
        .into_iter()
        .filter(|port| !allowlisted.contains(port.as_str()))
        .filter(|port| !contract_modules.contains(&to_snake_case(port)))
        .collect();

    assert!(
        missing.is_empty(),
        "public domain/application ports must have contract coverage in tests/contracts.rs \
         or an explicit allowlist entry; missing: {missing:?}"
    );
}

fn active_contract_modules() -> BTreeSet<String> {
    let contracts = fs::read_to_string("tests/contracts.rs").expect("read tests/contracts.rs");
    let path_re = regex::Regex::new(r#"^\s*#\[path\s*=\s*"contracts/([a-z0-9_]+)\.rs"\]\s*$"#)
        .expect("path regex");
    let mod_re = regex::Regex::new(r#"^\s*mod\s+([a-z0-9_]+)\s*;\s*$"#).expect("mod regex");
    let mut modules = BTreeSet::new();
    let mut pending_path: Option<String> = None;

    for line in contracts.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }
        if let Some(cap) = path_re.captures(trimmed) {
            pending_path = Some(cap[1].to_string());
            continue;
        }
        if let Some(cap) = mod_re.captures(trimmed) {
            let module = cap[1].to_string();
            if pending_path.as_deref() == Some(module.as_str()) {
                let path = format!("tests/contracts/{module}.rs");
                assert!(
                    Path::new(&path).exists(),
                    "contract module path must exist: {path}"
                );
                modules.insert(module);
            }
            pending_path = None;
            continue;
        }
        pending_path = None;
    }

    modules
}

fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (idx, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev_is_lower_or_digit = idx > 0
                && chars[idx - 1].is_ascii()
                && (chars[idx - 1].is_ascii_lowercase() || chars[idx - 1].is_ascii_digit());
            let next_is_lower = chars
                .get(idx + 1)
                .is_some_and(|next| next.is_ascii_lowercase());
            if idx > 0 && (prev_is_lower_or_digit || next_is_lower) {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[test]
fn pre_push_runs_contract_tests() {
    let content = fs::read_to_string("scripts/pre-push.sh").expect("read pre-push hook");
    assert!(
        content.contains("--test contracts"),
        "pre-push hook must run cargo test --test contracts so port contracts are enforced"
    );
}

#[test]
fn ci_runs_architecture_and_contract_tests() {
    let content = fs::read_to_string(".github/workflows/ci.yml").expect("read CI workflow");
    assert!(
        content.contains("--test architecture"),
        "CI must run architecture boundary tests"
    );
    assert!(
        content.contains("--test contracts"),
        "CI must run port contract tests"
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
