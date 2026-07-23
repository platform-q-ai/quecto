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

fn assert_no_inline_test_modules(crate_src: &Path) {
    let mut offenders = Vec::new();
    collect_rs_files(crate_src, &mut offenders);

    let offenders: Vec<_> = offenders
        .iter()
        .filter_map(|file_content| {
            let (file_path, content) = file_content.split_once(":\n").unwrap();
            (content.contains("#[cfg(test)]") && content.contains("mod tests {"))
                .then_some(file_path)
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "production files must keep tests in sibling *_tests.rs modules, not inline `mod tests` blocks:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn production_files_do_not_contain_inline_test_modules() {
    for crate_src in [
        "src",
        "../quecto-api/src",
        "../quecto-line-io/src",
        "../quecto-runtime-manager/src",
        "../quecto-tui/src",
    ] {
        assert_no_inline_test_modules(Path::new(crate_src));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TuiAllowlistEntry {
    path: &'static str,
    issue: &'static str,
}

// Temporary #1149/#1153/#1160 DTO-boundary exceptions: interface production
// code still speaks infrastructure::client DTOs until application/domain
// command and event vocabulary is introduced and mapped at the boundary.
const TUI_INTERFACE_PROTOCOL_DTO_ALLOWLIST: &[TuiAllowlistEntry] = &[
    TuiAllowlistEntry {
        path: "interface/app.rs",
        issue: "#1153",
    },
    TuiAllowlistEntry {
        path: "interface/app_effort.rs",
        issue: "#1153",
    },
    TuiAllowlistEntry {
        path: "interface/cli.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/app_events.rs",
        issue: "#1154",
    },
    TuiAllowlistEntry {
        path: "interface/app_ledger_sync.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_models.rs",
        issue: "#1153",
    },
    TuiAllowlistEntry {
        path: "interface/app_paged_history.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_response.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_feed.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_panel.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_state.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_stream.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagents.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_event_loop.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/app_message_recovery.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_methods.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_rewind.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_submit.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/chat.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/chat_render.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/footer.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/workflow_bar.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/feed_state.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/ledger_sync.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/range_accumulator.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness_disconnect.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness_events.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness_probes.rs",
        issue: "#1160",
    },
];

// Temporary #1149/#1155/#1156 use-case ownership exceptions: these interface
// modules still own orchestration/policy that should move into application
// services in the follow-up vertical slices.
const TUI_INTERFACE_USE_CASE_ALLOWLIST: &[TuiAllowlistEntry] = &[
    TuiAllowlistEntry {
        path: "interface/app_effort.rs",
        issue: "#1153",
    },
    TuiAllowlistEntry {
        path: "interface/app_event_loop.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/cli.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/app_events.rs",
        issue: "#1154",
    },
    TuiAllowlistEntry {
        path: "interface/app_events_test_support.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/app_ledger_sync.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_message_recovery.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_methods.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_models.rs",
        issue: "#1153",
    },
    TuiAllowlistEntry {
        path: "interface/app_paged_history.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_response.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_rewind.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/app_submit.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_feed.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_panel.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/app_subagent_stream.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/components/chat.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/chat_render.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/footer.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/components/workflow_bar.rs",
        issue: "#1162",
    },
    TuiAllowlistEntry {
        path: "interface/feed_state.rs",
        issue: "#1156",
    },
    TuiAllowlistEntry {
        path: "interface/ledger_sync.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/range_accumulator.rs",
        issue: "#1155",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness_events.rs",
        issue: "#1160",
    },
    TuiAllowlistEntry {
        path: "interface/tui_harness_probes.rs",
        issue: "#1160",
    },
];

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
fn tui_interface_protocol_dto_usage_is_allowlisted() {
    assert_tui_allowlist_entries_reference_issues(TUI_INTERFACE_PROTOCOL_DTO_ALLOWLIST);
    let offenders = tui_interface_files_containing(&[
        "crate::infrastructure::client",
        "infrastructure::client::",
        "use crate::infrastructure::client",
        "Command::",
        "SubagentInfoEvent",
        "SubagentWorkflow",
        "SyncDelta",
        "serde_json::Value",
    ]);
    assert_tui_offenders_are_allowlisted(
        &offenders,
        TUI_INTERFACE_PROTOCOL_DTO_ALLOWLIST,
        "protocol DTO",
    );
}

#[test]
fn tui_interface_use_case_ownership_is_allowlisted() {
    assert_tui_allowlist_entries_reference_issues(TUI_INTERFACE_USE_CASE_ALLOWLIST);
    let offenders = tui_interface_files_containing(&[
        "Command::",
        "Client::connect",
        "serde_json::from_value",
        "serde_json::from_str",
        "serde_json::to_string",
        "serde_json::json!",
        "serde_json::Value",
        "LedgerTranscript",
        "SyncDelta",
        "pending_rev",
        "supports_sync",
        "request_sync",
    ]);
    assert_tui_offenders_are_allowlisted(
        &offenders,
        TUI_INTERFACE_USE_CASE_ALLOWLIST,
        "use-case ownership",
    );
}

fn tui_interface_files_containing(patterns: &[&str]) -> BTreeSet<String> {
    let mut files = Vec::new();
    collect_rs_files(Path::new(TUI_INTERFACE), &mut files);
    files
        .iter()
        .filter_map(|file_content| {
            let (file_path, content) = file_content.split_once(":\n").unwrap();
            patterns
                .iter()
                .any(|pattern| content.contains(pattern))
                .then(|| {
                    Path::new(file_path)
                        .strip_prefix(TUI_SRC)
                        .expect("strip quecto-tui src prefix")
                        .to_string_lossy()
                        .replace('\\', "/")
                })
        })
        .collect()
}

fn assert_tui_offenders_are_allowlisted(
    offenders: &BTreeSet<String>,
    allowlist: &[TuiAllowlistEntry],
    category: &str,
) {
    let allowed: BTreeSet<_> = allowlist
        .iter()
        .map(|entry| entry.path.to_string())
        .collect();
    let missing: Vec<_> = offenders.difference(&allowed).cloned().collect();
    assert!(
        missing.is_empty(),
        "quecto-tui interface {category} hotspots must be moved inward or explicitly allowlisted with issue references: {missing:?}"
    );

    let stale: Vec<_> = allowed.difference(offenders).cloned().collect();
    assert!(
        stale.is_empty(),
        "quecto-tui interface {category} allowlist entries should be removed once no matching hotspot remains: {stale:?}"
    );
}

fn assert_tui_allowlist_entries_reference_issues(allowlist: &[TuiAllowlistEntry]) {
    assert!(!allowlist.is_empty(), "TUI allowlist must not be empty");
    for entry in allowlist {
        assert!(
            entry.issue.starts_with("#11") && entry.issue[1..].chars().all(|c| c.is_ascii_digit()),
            "TUI allowlist entry for {} must reference a #1149 child issue, got {}",
            entry.path,
            entry.issue
        );
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
            "cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings"
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

#[test]
fn normal_build_excludes_removed_tool_installer_archive_dependencies() {
    let manifest = fs::read_to_string("Cargo.toml").expect("read Cargo.toml");
    let normal_dependencies = cargo_toml_section(&manifest, "dependencies");

    for dependency in ["flate2", "tar"] {
        assert!(
            !cargo_toml_section_contains_dependency(normal_dependencies, dependency),
            "normal [dependencies] must not include `{dependency}` after removing the unused tool installer"
        );
    }

    assert!(
        !Path::new("src/infrastructure/tools/ensure_tool.rs").exists(),
        "removed tool installer source should not remain compiled into the harness"
    );

    let tools_mod = fs::read_to_string("src/infrastructure/tools/mod.rs")
        .expect("read tools module declarations");
    assert!(
        !tools_mod
            .lines()
            .any(|line| line.trim() == "pub mod ensure_tool;"),
        "removed tool installer should not be declared by the tools module"
    );
}

#[test]
fn macos_only_text_normalization_dependency_is_not_unconditional() {
    let manifest = fs::read_to_string("Cargo.toml").expect("read Cargo.toml");
    let normal_dependencies = cargo_toml_section(&manifest, "dependencies");
    assert!(
        !cargo_toml_section_contains_dependency(normal_dependencies, "unicode-normalization"),
        "unicode-normalization should not be pulled into non-macOS normal builds"
    );

    let macos_dependencies = cargo_toml_section(
        &manifest,
        "target.'cfg(target_os = \"macos\")'.dependencies",
    );
    assert!(
        cargo_toml_section_contains_dependency(macos_dependencies, "unicode-normalization"),
        "macOS path normalization should retain its unicode-normalization dependency"
    );
}

fn cargo_toml_section<'a>(manifest: &'a str, section: &str) -> &'a str {
    let header = format!("[{section}]");
    let Some(start) = manifest.find(&header) else {
        return "";
    };
    let after_header = &manifest[start + header.len()..];
    let end = after_header.find("\n[").unwrap_or(after_header.len());
    &after_header[..end]
}

fn cargo_toml_section_contains_dependency(section: &str, dependency: &str) -> bool {
    section.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('#')
            && (trimmed.starts_with(&format!("{dependency} ="))
                || trimmed.starts_with(&format!("{dependency}=")))
    })
}

/// #1059 / ADR-0008 part 1: all four socket consumers must migrate together.
/// Each production read site (harness UDS reader + multi-client reader,
/// sub-agent parent monitor, TUI client, quecto-api extension gateway) must
/// route through `quecto_line_io`'s deprecation-window frame reader, so a
/// partial migration (one consumer silently left on bare newline reads)
/// cannot land.
#[test]
fn all_four_socket_consumers_read_via_the_shared_frame_reader() {
    let consumers = [
        "src/interface/cli/uds_reader.rs",
        "src/interface/cli/uds_multi.rs",
        "src/infrastructure/tools/subagent_monitor.rs",
        // The parent→child query path (agent tool + TUI inspector poll) is a
        // socket consumer too; it must not be stranded on legacy NDJSON reads
        // when the deprecation window closes (#1059 review, finding 5).
        "src/infrastructure/tools/subagent_registry.rs",
        "../quecto-tui/src/infrastructure/client.rs",
        "../quecto-api/src/infrastructure/uds/client.rs",
    ];
    for path in consumers {
        let source =
            fs::read_to_string(path).unwrap_or_else(|e| panic!("read consumer source {path}: {e}"));
        assert!(
            source.contains("read_frame_or_legacy_line"),
            "{path} must read socket messages via \
             quecto_line_io::read_frame_or_legacy_line (#1059)"
        );
    }
}

/// #1059 / ADR-0008 part 1: the agent's socket announcement must carry the
/// protocol version, and the production multi-client emit site must produce it
/// via the shared `socket_announcement` helper (the single source of truth for
/// the `quecto-agent-protocol: N` line a client sniffs before connecting).
///
/// The BDD announcement scenario asserts on `socket_announcement`'s output, so
/// on its own it cannot catch the emit site being reverted to a bare
/// `eprintln!("quecto-agent-socket: …")` that drops the version line. This
/// source guard pins that the production path actually emits the version line
/// through the helper, so such a revert fails a test (#1059 review, finding 9).
#[test]
fn multi_client_agent_announces_protocol_version_via_shared_helper() {
    let source = fs::read_to_string("src/interface/cli/uds_lifecycle.rs")
        .expect("read src/interface/cli/uds_lifecycle.rs");
    assert!(
        source.contains("socket_announcement(&socket_path)"),
        "the multi-client agent startup must emit its socket announcement via \
         uds_wire::socket_announcement (which carries the protocol-version line); \
         a bare eprintln! that drops the version line would silently strand \
         clients on legacy framing (#1059)"
    );
    // And the helper itself must include the version token, so the emitted
    // announcement is never just the legacy socket line.
    let wire = fs::read_to_string("src/interface/cli/uds_wire.rs")
        .expect("read src/interface/cli/uds_wire.rs");
    assert!(
        wire.contains("PROTOCOL_ANNOUNCE_PREFIX") && wire.contains("PROTOCOL_VERSION"),
        "socket_announcement must include the PROTOCOL_ANNOUNCE_PREFIX + PROTOCOL_VERSION token"
    );
}
