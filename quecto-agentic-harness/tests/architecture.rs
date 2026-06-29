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

const TUI_SRC: &str = "../quecto-tui/src";
const TUI_DOMAIN: &str = "../quecto-tui/src/domain";
const TUI_APPLICATION: &str = "../quecto-tui/src/application";
const TUI_INFRASTRUCTURE: &str = "../quecto-tui/src/infrastructure";
const TUI_INTERFACE: &str = "../quecto-tui/src/interface";
const TUI_ALLOWED_ROOT_RS: &[&str] = &["lib.rs", "main.rs"];

#[test]
fn tui_architecture_layers_exist() {
    assert!(
        Path::new(TUI_DOMAIN).exists(),
        "quecto-tui/src/domain/ must exist"
    );
    assert!(
        Path::new(TUI_APPLICATION).exists(),
        "quecto-tui/src/application/ must exist"
    );
    assert!(
        Path::new(TUI_INFRASTRUCTURE).exists(),
        "quecto-tui/src/infrastructure/ must exist"
    );
    assert!(
        Path::new(TUI_INTERFACE).exists(),
        "quecto-tui/src/interface/ must exist"
    );
}

#[test]
fn tui_domain_has_no_outer_layer_imports() {
    assert_no_imports(
        "quecto-tui domain",
        Path::new(TUI_DOMAIN),
        &[
            "crate::application",
            "crate::infrastructure",
            "crate::interface",
            "super::application",
            "super::infrastructure",
            "super::interface",
        ],
    );
}

#[test]
fn tui_application_has_no_infrastructure_or_interface_imports() {
    assert_no_imports(
        "quecto-tui application",
        Path::new(TUI_APPLICATION),
        &[
            "crate::infrastructure",
            "crate::interface",
            "super::infrastructure",
            "super::interface",
        ],
    );
}

#[test]
fn tui_infrastructure_has_no_application_or_interface_imports() {
    assert_no_imports(
        "quecto-tui infrastructure",
        Path::new(TUI_INFRASTRUCTURE),
        &[
            "crate::application",
            "crate::interface",
            "super::application",
            "super::interface",
        ],
    );
}

#[test]
fn tui_inner_layers_have_no_runtime_io_calls() {
    let runtime_io = [
        "std::fs::",
        "tokio::fs::",
        "std::env::",
        "dirs::",
        ".exists(",
    ];
    assert_no_imports("quecto-tui domain", Path::new(TUI_DOMAIN), &runtime_io);
    assert_no_imports(
        "quecto-tui application",
        Path::new(TUI_APPLICATION),
        &runtime_io,
    );
}

#[test]
fn tui_runtime_adapters_live_in_infrastructure() {
    for adapter in ["client", "process", "render", "signals", "terminal"] {
        let infrastructure_path = format!("{TUI_INFRASTRUCTURE}/{adapter}.rs");
        let interface_path = format!("{TUI_INTERFACE}/{adapter}.rs");
        assert!(
            Path::new(&infrastructure_path).is_file(),
            "TUI runtime adapter must live in infrastructure: {infrastructure_path}"
        );
        assert!(
            !Path::new(&interface_path).exists(),
            "TUI runtime adapter must not live in interface: {interface_path}"
        );
    }
}

#[test]
fn tui_production_files_live_inside_architecture_layers() {
    let mut misplaced = Vec::new();
    collect_misplaced_tui_rs_files(Path::new(TUI_SRC), &mut misplaced);
    assert!(
        misplaced.is_empty(),
        "quecto-tui production Rust files must live under domain/, application/, infrastructure/, or interface/; misplaced: {misplaced:?}"
    );
}

fn collect_misplaced_tui_rs_files(dir: &Path, misplaced: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read quecto-tui src dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_misplaced_tui_rs_files(&path, misplaced);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let rel = path
            .strip_prefix(TUI_SRC)
            .expect("strip quecto-tui src prefix")
            .to_string_lossy()
            .replace('\\', "/");
        let top = rel.split('/').next().unwrap_or_default();
        let in_layer = matches!(
            top,
            "domain" | "application" | "infrastructure" | "interface"
        );
        let allowed_root = !rel.contains('/') && TUI_ALLOWED_ROOT_RS.contains(&rel.as_str());
        if !in_layer && !allowed_root {
            misplaced.push(rel);
        }
    }
}

#[test]
fn tui_public_ports_have_contract_tests() {
    let mut files = Vec::new();
    collect_rs_files(Path::new(TUI_DOMAIN), &mut files);
    collect_rs_files(Path::new(TUI_APPLICATION), &mut files);

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
        .filter(|port| !contract_modules.contains(&format!("tui_{}", to_snake_case(port))))
        .collect();

    assert!(
        missing.is_empty(),
        "public quecto-tui domain/application ports must have contract coverage in tests/contracts.rs with a tui_ prefix or an explicit allowlist entry; missing: {missing:?}"
    );
}

#[test]
fn tui_lib_rs_exposes_only_architecture_layers() {
    let content = fs::read_to_string("../quecto-tui/src/lib.rs").expect("read quecto-tui lib.rs");
    let public_modules: Vec<_> = content
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("pub mod "))
        .map(|rest| rest.trim_end_matches(';'))
        .collect();
    assert_eq!(
        public_modules,
        ["application", "domain", "infrastructure", "interface"],
        "quecto-tui/src/lib.rs must expose only Clean Architecture layer modules"
    );
    assert!(
        !content.contains("#[path ="),
        "quecto-tui/src/lib.rs must not re-export interface internals with #[path] shims"
    );
}

#[test]
fn tui_main_rs_is_thin_interface_entrypoint() {
    let content = fs::read_to_string("../quecto-tui/src/main.rs").expect("read quecto-tui main.rs");
    assert!(
        content.contains("quecto_tui::interface::cli") && content.lines().count() <= 10,
        "quecto-tui/src/main.rs must stay thin and delegate to quecto_tui::interface::cli"
    );
}

#[test]
fn tui_lib_rs_has_deny_attributes() {
    let content = fs::read_to_string("../quecto-tui/src/lib.rs").expect("read quecto-tui lib.rs");
    assert!(
        content.contains("#![deny(dead_code)]"),
        "quecto-tui/src/lib.rs must contain #![deny(dead_code)]"
    );
    assert!(
        content.contains("#![deny(unused_imports)]"),
        "quecto-tui/src/lib.rs must contain #![deny(unused_imports)]"
    );
}

#[test]
fn pre_commit_enforces_formatting_and_complexity_lints() {
    assert_local_hook_enforces_formatting_and_complexity("pre-commit");
}

#[test]
fn pre_push_enforces_formatting_and_complexity_lints() {
    assert_local_hook_enforces_formatting_and_complexity("pre-push");
}

fn assert_local_hook_enforces_formatting_and_complexity(hook: &str) {
    let path = format!("../scripts/{hook}.sh");
    let content = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path}: {err}"));
    assert!(
        content.contains("cargo fmt --all -- --check"),
        "{hook} hook must enforce rustfmt max_width locally"
    );
    assert!(
        content.contains(
            "cargo clippy --workspace --all-targets --features quecto/test-support -- -D warnings"
        ),
        "{hook} hook must run strict workspace clippy locally"
    );
    for lint in [
        "-W clippy::cognitive_complexity",
        "-W clippy::too_many_arguments",
        "-W clippy::too_many_lines",
    ] {
        assert!(
            content.contains(lint),
            "{hook} hook must enforce complexity lint: {lint}"
        );
    }
}

#[test]
fn hook_installation_checker_verifies_all_local_hooks() {
    let content = fs::read_to_string("../scripts/check-hooks-installed.sh")
        .expect("read hook installation checker");
    for hook in ["pre-commit", "pre-push"] {
        assert!(
            content.contains(hook),
            "hook checker must verify {hook} is installed"
        );
    }
    assert!(
        content.contains("command -v git"),
        "hook checker must verify the git wrapper is active in PATH"
    );
}

#[test]
fn pre_push_runs_contract_tests() {
    let content = fs::read_to_string("../scripts/pre-push.sh").expect("read pre-push hook");
    assert!(
        content.contains("--test contracts"),
        "pre-push hook must run cargo test --test contracts so port contracts are enforced"
    );
}

#[test]
fn ci_runs_architecture_and_contract_tests() {
    let content = fs::read_to_string("../.github/workflows/ci.yml").expect("read CI workflow");
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

#[test]
fn runtime_manager_domain_is_pure() {
    // The quecto-runtime-manager crate has its own domain/application/infrastructure
    // split; its domain must stay free of outward (infra/app/framework) deps.
    let src = fs::read_to_string("../quecto-runtime-manager/src/domain.rs")
        .expect("read quecto-runtime-manager/src/domain.rs");
    // Scan production code only — stop at the test module.
    let prod = src.split("#[cfg(test)]").next().unwrap_or(&src);
    for pattern in [
        "crate::infrastructure",
        "crate::application",
        "reqwest",
        "tokio",
        "axum",
        "kube",
    ] {
        assert!(
            !prod.contains(pattern),
            "quecto-runtime-manager/src/domain.rs must stay pure: found `{pattern}`"
        );
    }
}
