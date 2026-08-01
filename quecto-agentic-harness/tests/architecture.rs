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
const TUI_CONVERSATION: &str = "../quecto-tui/src/conversation";
const TUI_AGENTS: &str = "../quecto-tui/src/agents";
const TUI_PROTOCOL: &str = "../quecto-tui/src/protocol";
const TUI_SESSIONS: &str = "../quecto-tui/src/sessions";
const TUI_WORKFLOW: &str = "../quecto-tui/src/workflow";
const TUI_INFERENCE: &str = "../quecto-tui/src/inference";
const TUI_WORKSPACE: &str = "../quecto-tui/src/workspace";
const TUI_COMPONENTS: &str = "../quecto-tui/src/components";
const TUI_SHELL: &str = "../quecto-tui/src/shell";
const TUI_ALLOWED_ROOT_RS: &[&str] = &["lib.rs", "main.rs"];
/// #1257 Phase 6 final module set: feature-oriented presentation modules only.
const TUI_LIB_RS_MODULES: &[&str] = &[
    "agents",
    "components",
    "conversation",
    "inference",
    "protocol",
    "sessions",
    "shell",
    "workflow",
    "workspace",
];
/// #1257 Phase 6: production files may live only under feature-oriented modules.
const TUI_TOP_LEVEL_MODULES: &[&str] = &[
    "agents",
    "components",
    "conversation",
    "inference",
    "protocol",
    "sessions",
    "shell",
    "workflow",
    "workspace",
];
const TUI_FEATURE_ARCH_DOC: &str =
    "../quecto-tui/docs/feature-oriented-presentation-architecture.md";
const TUI_SUPERSEDED_ARCH_DOC: &str = "../quecto-tui/docs/clean-architecture-target-model.md";
const TUI_README: &str = "../quecto-tui/README.md";

#[test]
fn tui_feature_oriented_architecture_is_documented() {
    let feature_doc = fs::read_to_string(TUI_FEATURE_ARCH_DOC)
        .expect("read quecto-tui feature-oriented architecture doc");
    assert!(
        feature_doc.contains("Feature-oriented presentation architecture"),
        "feature-oriented TUI architecture doc must have the expected title"
    );
    for capability in [
        "shell",
        "protocol",
        "conversation",
        "sessions",
        "agents",
        "workflow",
        "inference",
        "workspace",
        "components",
    ] {
        let target_bullet = format!("- `{capability}`:");
        assert!(
            feature_doc.contains(&target_bullet),
            "feature-oriented TUI architecture doc must include target capability bullet {target_bullet:?}"
        );
    }
    for required in [
        "Raw UDS framing",
        "raw JSON interpretation",
        "Pure policy modules must not depend on terminal/widget types",
        "Do not introduce a second global command/event hierarchy",
        "Capability characterization and migration map",
        "Production file target-owner map",
        "`components/autocomplete.rs` | `components` (relocated, #1257 Phase 1)",
        "`components/workflow_bar.rs` | `components` (relocated, #1257 Phase 1)",
        "`components/ansi.rs` | `components` rendering primitive (relocated, #1257 Phase 6)",
        "`components/theme.rs` | `components` styling primitive (relocated, #1257 Phase 6)",
        "`shell/app.rs` | `shell` composition root (relocated, #1257 Phase 6)",
        "`shell/stdin_buffer.rs` | `shell` stdin adapter/policy (relocated, #1257 Phase 6)",
        "`conversation/history_paging.rs` | `conversation` history cursors, page correlation and backfill latch (#1221; relocated, #1257 Phase 3)",
        "`conversation/controller_rewind.rs` | `conversation` rewind flow owner (relocated, #1257 Phase 3)",
        "`domain/` | deleted in #1257 Phase 3",
        "`agents/roster.rs` | `agents` pure roster/lifecycle policy (#1222; relocated, #1257 Phase 4)",
        "`agents/controller_subagents.rs` | `agents` (relocated, #1257 Phase 4)",
        "`agents/runtime.rs` | `agents` feed connect-task runtime ownership (#1257 Phase 4 ui.rs split)",
        "`agents/view.rs` | `agents` concrete UI/runtime adapter state (#1222; relocated + split, #1257 Phase 4)",
        "`sessions/controller_sessions.rs` | `sessions` (relocated, #1257 Phase 5)",
        "`workflow/controller_workflow.rs` | `workflow` (relocated, #1257 Phase 5)",
        "`inference/controller_models.rs` | `inference` (relocated, #1257 Phase 5)",
        "`workspace/workspace_files.rs` | `workspace` (relocated, #1257 Phase 5)",
        "`infrastructure/` | deleted in #1257 Phase 5",
        "`interface/` | deleted in #1257 Phase 6",
        "#1221 (`conversation`) and #1222 (`agents`) depend on #1220",
    ] {
        assert!(
            feature_doc.contains(required),
            "feature-oriented TUI architecture doc must include {required:?}"
        );
    }
    assert!(
        !feature_doc.contains("## Interim compatibility map"),
        "feature-oriented TUI architecture doc must not retain the interim compatibility map after #1257 Phase 6"
    );

    let mapped_files = production_owner_map_files(&feature_doc);
    let current_files = current_tui_production_files();
    assert_eq!(
        mapped_files, current_files,
        "feature-oriented TUI production owner map must exactly match current production files"
    );

    let superseded_doc =
        fs::read_to_string(TUI_SUPERSEDED_ARCH_DOC).expect("read old quecto-tui architecture doc");
    assert!(
        superseded_doc.contains("SUPERSEDED")
            && superseded_doc.contains("feature-oriented-presentation-architecture.md"),
        "old TUI architecture target model must be marked superseded and point to the current doc"
    );

    let readme = fs::read_to_string(TUI_README).expect("read quecto-tui README");
    assert!(
        readme.contains("feature-oriented presentation adapter")
            && readme.contains("docs/feature-oriented-presentation-architecture.md")
            && readme.contains("superseded historical context"),
        "TUI README must point readers to the current feature-oriented architecture doc"
    );
}

fn production_owner_map_files(feature_doc: &str) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    let mut in_map = false;
    for line in feature_doc.lines() {
        if line == "### Production file target-owner map" {
            in_map = true;
            continue;
        }
        if in_map && line.starts_with("### ") {
            break;
        }
        if !in_map || line.starts_with("|---") || line.starts_with("| Current production file") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("| `") {
            if let Some((file, _)) = rest.split_once('`') {
                files.insert(file.to_string());
            }
        }
    }
    files
}

fn current_tui_production_files() -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    collect_tui_production_rs_paths(Path::new(TUI_SRC), &mut files);
    files
}

fn collect_tui_production_rs_paths(dir: &Path, files: &mut BTreeSet<String>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_tui_production_rs_paths(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
        {
            files.insert(
                path.strip_prefix(TUI_SRC)
                    .expect("TUI source path is below TUI src root")
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
}

#[test]
fn tui_architecture_layers_exist() {
    assert!(
        Path::new(TUI_CONVERSATION).exists(),
        "quecto-tui/src/conversation/ must exist after #1257 Phase 3"
    );
    assert!(
        Path::new(TUI_AGENTS).exists(),
        "quecto-tui/src/agents/ must exist after #1257 Phase 4"
    );
    assert!(
        !Path::new("../quecto-tui/src/interface/agents").exists(),
        "quecto-tui/src/interface/agents/ must be promoted to top-level agents/ after #1257 Phase 4"
    );
    assert!(
        Path::new(TUI_PROTOCOL).exists(),
        "quecto-tui/src/protocol/ must exist"
    );
    assert!(
        Path::new(TUI_SESSIONS).exists(),
        "quecto-tui/src/sessions/ must exist after #1257 Phase 5"
    );
    assert!(
        Path::new(TUI_WORKFLOW).exists(),
        "quecto-tui/src/workflow/ must exist after #1257 Phase 5"
    );
    assert!(
        Path::new(TUI_INFERENCE).exists(),
        "quecto-tui/src/inference/ must exist after #1257 Phase 5"
    );
    assert!(
        Path::new(TUI_WORKSPACE).exists(),
        "quecto-tui/src/workspace/ must exist after #1257 Phase 5"
    );
    assert!(
        !Path::new("../quecto-tui/src/infrastructure").exists(),
        "quecto-tui/src/infrastructure/ must be deleted after #1257 Phase 5"
    );
    assert!(
        Path::new(TUI_SHELL).exists(),
        "quecto-tui/src/shell/ must exist"
    );
    assert!(
        Path::new(TUI_COMPONENTS).exists(),
        "quecto-tui/src/components/ must exist"
    );
    assert!(
        !Path::new("../quecto-tui/src/interface").exists(),
        "quecto-tui/src/interface/ must be deleted after #1257 Phase 6"
    );
    assert!(
        !Path::new("../quecto-tui/src/application").exists(),
        "quecto-tui/src/application/ must be deleted after #1257 Phase 2"
    );
    assert!(
        !Path::new("../quecto-tui/src/domain").exists(),
        "quecto-tui/src/domain/ must be deleted after #1257 Phase 3"
    );
}

#[test]
fn tui_conversation_pure_policy_has_no_outer_layer_imports() {
    let forbidden = [
        "crate::application",
        "crate::infrastructure",
        "crate::interface",
        "crate::protocol",
        "crate::components",
        "crate::shell",
        "super::application",
        "super::infrastructure",
        "super::interface",
        "super::protocol",
    ];
    for rel in ["history_paging.rs", "turn_recovery.rs"] {
        let path = Path::new(TUI_CONVERSATION).join(rel);
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read pure conversation policy {}: {e}", path.display()));
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") {
                continue;
            }
            for pattern in forbidden {
                assert!(
                    !trimmed.contains(pattern),
                    "pure conversation policy {} must not import outer layer pattern {pattern}; line: {trimmed}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn tui_agents_pure_policy_has_no_outer_layer_imports() {
    // Pure agents policy (#1222): feed/focus/roster must not depend on
    // terminal/widget types, concrete client, or outer presentation layers.
    // `ledger.rs` is a typed adapter over protocol ledger payloads + conversation
    // recovery ordering and is checked separately for widget/client freedom.
    let forbidden = [
        "crate::application",
        "crate::infrastructure",
        "crate::interface",
        "crate::components",
        "crate::shell",
        "crate::protocol::client",
        "super::application",
        "super::infrastructure",
        "super::interface",
        "super::components",
        "super::shell",
        "mpsc::",
        "JoinHandle",
        "Client::",
        "serde_json::",
    ];
    for rel in ["feed.rs", "focus.rs", "roster.rs"] {
        let path = Path::new(TUI_AGENTS).join(rel);
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read pure agents policy {}: {e}", path.display()));
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") {
                continue;
            }
            for pattern in forbidden {
                assert!(
                    !trimmed.contains(pattern),
                    "pure agents policy {} must not contain pattern {pattern}; line: {trimmed}",
                    path.display()
                );
            }
        }
    }
    let ledger =
        fs::read_to_string(Path::new(TUI_AGENTS).join("ledger.rs")).expect("read agents ledger");
    for pattern in [
        "crate::interface",
        "crate::components",
        "crate::shell",
        "crate::protocol::client",
        "mpsc::",
        "JoinHandle",
        "Client::",
        "serde_json::",
    ] {
        for line in ledger.lines() {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") {
                continue;
            }
            assert!(
                !trimmed.contains(pattern),
                "agents ledger must not contain pattern {pattern}; line: {trimmed}"
            );
        }
    }
}

#[test]
fn tui_workspace_files_adapter_does_not_import_presentation_layers() {
    // #1257 Phase 5: workspace_files moved out of infrastructure/; it may use
    // std/process IO but must not depend on presentation modules.
    let path = Path::new(TUI_WORKSPACE).join("workspace_files.rs");
    let content = fs::read_to_string(&path).expect("read workspace_files");
    for pattern in [
        "crate::interface",
        "crate::components",
        "crate::shell",
        "crate::protocol",
        "crate::conversation",
        "crate::agents",
        "crate::sessions",
        "crate::workflow",
        "crate::inference",
    ] {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") {
                continue;
            }
            assert!(
                !trimmed.contains(pattern),
                "workspace_files must not import {pattern}; line: {trimmed}"
            );
        }
    }
}

#[test]
fn tui_protocol_has_no_feature_or_shell_imports() {
    // #1257 Phase 2: protocol owns the UDS client + mappers and must not
    // depend outward on presentation modules.
    assert_no_imports(
        "quecto-tui protocol",
        Path::new(TUI_PROTOCOL),
        &[
            "crate::components",
            "crate::shell",
            "crate::interface",
            "crate::conversation",
            "crate::sessions",
            "crate::agents",
            "crate::workflow",
            "crate::inference",
            "crate::workspace",
            "super::components",
            "super::shell",
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
    assert_no_imports(
        "quecto-tui conversation",
        Path::new(TUI_CONVERSATION),
        &runtime_io,
    );
}

#[test]
fn tui_runtime_adapters_live_in_shell() {
    // #1257 Phase 2: the UDS client lives in `protocol/`; terminal/runtime
    // adapters live in `shell/`.
    for (root, adapter) in [
        (TUI_PROTOCOL, "client"),
        (TUI_SHELL, "process"),
        (TUI_SHELL, "render"),
        (TUI_SHELL, "signals"),
        (TUI_SHELL, "terminal"),
        (TUI_SHELL, "child_watch"),
        (TUI_SHELL, "warn_capture"),
    ] {
        let adapter_path = format!("{root}/{adapter}.rs");
        let deleted_interface_path = format!("../quecto-tui/src/interface/{adapter}.rs");
        assert!(
            Path::new(&adapter_path).is_file(),
            "TUI runtime adapter must live in its owning module: {adapter_path}"
        );
        assert!(
            !Path::new(&deleted_interface_path).exists(),
            "TUI runtime adapter must not live in deleted interface/: {deleted_interface_path}"
        );
    }
}

#[test]
fn tui_production_files_live_inside_architecture_layers() {
    let mut misplaced = Vec::new();
    collect_misplaced_tui_rs_files(Path::new(TUI_SRC), &mut misplaced);
    assert!(
        misplaced.is_empty(),
        "quecto-tui production Rust files must live under a recognised top-level module ({TUI_TOP_LEVEL_MODULES:?}); misplaced: {misplaced:?}"
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
        let in_layer = TUI_TOP_LEVEL_MODULES.contains(&top);
        let allowed_root = !rel.contains('/') && TUI_ALLOWED_ROOT_RS.contains(&rel.as_str());
        if !in_layer && !allowed_root {
            misplaced.push(rel);
        }
    }
}

#[test]
fn tui_public_ports_have_contract_tests() {
    let mut files = Vec::new();
    collect_rs_files(Path::new(TUI_CONVERSATION), &mut files);
    collect_rs_files(Path::new(TUI_PROTOCOL), &mut files);

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
        "public quecto-tui conversation/protocol ports must have contract coverage in tests/contracts.rs with a tui_ prefix or an explicit allowlist entry; missing: {missing:?}"
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
        public_modules, TUI_LIB_RS_MODULES,
        "quecto-tui/src/lib.rs must expose exactly the per-phase module set (#1257)"
    );
    assert!(
        !content.contains("#[path ="),
        "quecto-tui/src/lib.rs must not re-export feature internals with #[path] shims"
    );
}

#[test]
fn tui_main_rs_is_thin_shell_entrypoint() {
    let content = fs::read_to_string("../quecto-tui/src/main.rs").expect("read quecto-tui main.rs");
    assert!(
        content.contains("quecto_tui::shell::cli") && content.lines().count() <= 10,
        "quecto-tui/src/main.rs must stay thin and delegate to quecto_tui::shell::cli"
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
fn local_hooks_enforce_formatting_while_ci_owns_strict_lints() {
    for hook in ["pre-commit", "pre-push"] {
        let path = format!("../scripts/{hook}.sh");
        let content = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        assert!(
            content.contains("cargo fmt --all -- --check"),
            "{hook} hook must enforce rustfmt locally"
        );
    }

    let ci = fs::read_to_string("../.github/workflows/ci.yml").expect("read CI workflow");
    assert!(
        ci.contains(
            "cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings"
        ),
        "authoritative CI must run strict workspace clippy"
    );
    for lint in [
        "-W clippy::cognitive_complexity",
        "-W clippy::too_many_arguments",
        "-W clippy::too_many_lines",
    ] {
        assert!(ci.contains(lint), "CI must enforce complexity lint: {lint}");
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
        "../quecto-tui/src/protocol/client.rs",
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

// ── #1220 protocol-boundary ratchets ────────────────────────────────
//
// Two decrease-only ratchets guard the raw-JSON burn-down. Seeds may be lowered
// as sites migrate to protocol-layer mappers; they may never be raised.
//
// Exclusion is content-based, not filename-based: a file is treated as test
// code only if it carries `cfg(test)` or test attributes, so `*_test_support.rs`
// fixtures cannot inflate the production seed, while feature-gated PRODUCTION
// modules (`tui_harness*.rs`, gated by the `test-harness` *feature*, not
// `cfg(test)`) are now correctly measured rather than silently exempt.
//
// Allowlisting is per-ratchet, so an exemption is never broader than its
// stated rationale.

/// Seed: production raw `serde_json` parsing sites still resident in TUI
/// feature/view modules. Lower this as call sites migrate
/// behind mappers. Never raise it.
const TUI_FEATURE_VIEW_RAW_JSON_SITE_SEED: usize = 0;
/// Measured after #1257 Phase 5 relocation plus genuine get_state/workflow/
/// set_effort mapper conversions; future relocations must not burn down sites
/// by moving scan roots alone.
const TUI_PHASE_6_FEATURE_VIEW_RAW_JSON_TOTAL: usize = 0;

/// #1257: feature/view ratchet scan roots follow the code as modules relocate,
/// so a move alone can never lower a measured count (Phase 5: four new feature
/// modules plus remaining `interface/`/`components/`/`shell/`).
/// #1257 Phase 6 final feature/view scan roots (no interface/).
const TUI_FEATURE_VIEW_RATCHET_ROOTS: &[&str] = &[
    TUI_COMPONENTS,
    TUI_SHELL,
    TUI_CONVERSATION,
    TUI_AGENTS,
    TUI_SESSIONS,
    TUI_WORKFLOW,
    TUI_INFERENCE,
    TUI_WORKSPACE,
];

/// Seed: production raw `serde_json` parsing sites in TUI `protocol/` mappers.
/// These sites are an allowed but temporary protocol-mapping foothold; lower
/// this as they migrate behind typed mappers. Never raise it.
/// (#1257 Phase 5: raised only by genuine new mapper sites absorbed from
/// feature/view — net feature-view burn-down is required when raising.)
const TUI_PROTOCOL_RAW_JSON_SITE_SEED: usize = 126;
/// Measured with key, indexed-value, and accessor-chain parsing all counted.
const TUI_PHASE_6_PROTOCOL_RAW_JSON_TOTAL: usize = 126;
/// Historical combined feature/view + protocol ceiling. This prevents moving
/// sites between buckets (and adjusting their individual seeds) from hiding
/// growth in the total raw-JSON inventory.
const TUI_RAW_JSON_COMBINED_CEILING: usize = 178;

/// Seed: production feature/view *usages* of `protocol::client` wire DTOs.
/// Lower this as call sites migrate behind mappers. Never raise it.
const TUI_WIRE_DTO_USAGE_SEED: usize = 97;
/// Measured after #1257 Phase 6, including the documented transport seams.
/// #1259 folded child stream Event matches while adding live-buffer retention.
/// The child-progress-freeze fix (2026-07-29) added two test-harness probe
/// sites in `shell/tui_harness_probes.rs` (full-channel child feed setup for
/// the refused-Sync BDD scenarios) — probe-only, no production growth.
const TUI_PHASE_6_WIRE_DTO_USAGE_TOTAL: usize = 116;

/// Narrow, issue-linked allowlist for the INTERFACE RAW-JSON ratchet only.
///
/// The response dispatcher IS the protocol seam: it receives raw responses and
/// routes them to mappers, so raw JSON access there is by construction. It is
/// deliberately NOT exempt from the wire-DTO ratchet.
const TUI_FEATURE_VIEW_RAW_JSON_ALLOWLIST: &[(&str, &str)] =
    &[("shell/app_response.rs", "#1220 response dispatch seam")];

/// Narrow, issue-linked allowlist for the PROTOCOL RAW-JSON ratchet only.
/// The UDS client is the wire seam itself (frame/event field access); mapper
/// sites remain measured. (#1257 Phase 2)
const TUI_PROTOCOL_RAW_JSON_ALLOWLIST: &[(&str, &str)] = &[("protocol/client.rs", "#1257")];

/// Wire DTO seam files remain in the measured inventory. Keeping this
/// allowlist empty ensures that new DTO usage inside a documented seam still
/// increases the decrease-only ratchet rather than receiving a whole-file
/// exemption.
const TUI_WIRE_DTO_ALLOWLIST: &[(&str, &str)] = &[];

/// Production (non-`cfg(test)`) TUI layer sources, minus `allowlist`.
fn tui_production_layer_files(root: &str, allowlist: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect_tui_production_layer_files(Path::new(root), &mut out);
    out.retain(|(rel, _)| !allowlist.iter().any(|(suffix, _)| rel.ends_with(suffix)));
    out.sort();
    out
}

fn collect_tui_production_layer_files(dir: &Path, out: &mut Vec<(String, String)>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read quecto-tui layer dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_tui_production_layer_files(&path, out);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".rs") {
            continue;
        }
        let content = fs::read_to_string(&path).expect("read tui source");
        // cfg(test)-based, not name-based: only genuine test modules are exempt.
        if is_test_module(&content) {
            continue;
        }
        let rel = path
            .to_string_lossy()
            .replace("../quecto-tui/src/", "")
            .replace('\\', "/");
        out.push((rel, content));
    }
}

/// True when the file is test code: either compiled only under `cfg(test)`, or
/// a test module body (sibling `#[path]` files carry the test attributes while
/// the `#[cfg(test)]` gate sits on the parent's `mod` declaration).
fn is_test_module(content: &str) -> bool {
    content.lines().any(|line| {
        let t = line.trim();
        // NOTE: a bare `#[cfg(test)]` is NOT enough — production modules carry
        // one on their trailing `mod tests;` declaration. Only a whole-file
        // gate or an actual test body counts.
        t == "#![cfg(test)]"
            || t == "#[test]"
            || t.starts_with("#[tokio::test")
            || t.starts_with("#[rstest")
    })
}

/// Raw JSON parsing = reaching into a `serde_json::Value` by field or shape.
///
/// Accessors like `as_str()` exist on plain `String` too, so a bare accessor is
/// only counted when the line also shows a key lookup (`.get("…")`/`.pointer("…")`)
/// or an `and_then` chain, which is how `serde_json` access actually reads. This
/// keeps `args[i].as_str()` and `m.id.as_str()` out of the inventory.
fn raw_json_site_count(content: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let t = line.trim();
            if t.starts_with("//") || t.starts_with("///") {
                return false;
            }
            let keys = t.contains(".get(\"") || t.contains(".pointer(\"");
            let accessor = t.contains("as_array()")
                || t.contains("as_object()")
                || t.contains("as_str()")
                || t.contains("as_u64()")
                || t.contains("as_i64()")
                || t.contains("as_bool()");
            let indexed_value = t.contains("[\"") && accessor;
            keys || indexed_value || (accessor && t.contains("and_then"))
        })
        .count()
}

/// Uses of wire DTOs, not merely `use` lines: `use super::*` re-exports and
/// fully-qualified paths would otherwise make the import count meaningless.
fn wire_dto_usage_count(content: &str) -> usize {
    let imports_client = content
        .lines()
        .any(|line| line.contains("protocol::client"));
    content
        .lines()
        .filter(|line| {
            let t = line.trim();
            if t.starts_with("//") || t.starts_with("///") {
                return false;
            }
            t.contains("protocol::client")
                || (imports_client
                    && ["Command", "Event", "SubagentInfoEvent", "SubagentWorkflow"]
                        .iter()
                        .any(|name| t.contains(name)))
        })
        .count()
}

fn tui_ratchet_inventory(
    roots: &[&str],
    allowlist: &[(&str, &str)],
    count: fn(&str) -> usize,
) -> (usize, Vec<(String, usize)>) {
    let mut files = Vec::new();
    for root in roots {
        let root_files = tui_production_layer_files(root, allowlist);
        assert!(
            !root_files.is_empty(),
            "the TUI scan root {root} yielded no production files; \
             a path rename must not silently disable the #1220 ratchets"
        );
        files.extend(root_files);
    }
    let mut per_file: Vec<(String, usize)> = files
        .iter()
        .map(|(rel, content)| (rel.clone(), count(content)))
        .filter(|(_, n)| *n > 0)
        .collect();
    per_file.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let total = per_file.iter().map(|(_, n)| n).sum();
    (total, per_file)
}

#[test]
fn tui_feature_view_raw_json_parsing_sites_are_eliminated() {
    let (total, per_file) = tui_ratchet_inventory(
        TUI_FEATURE_VIEW_RATCHET_ROOTS,
        TUI_FEATURE_VIEW_RAW_JSON_ALLOWLIST,
        raw_json_site_count,
    );
    assert_eq!(
        total, TUI_PHASE_6_FEATURE_VIEW_RAW_JSON_TOTAL,
        "#1257 Phase 6 relocation must preserve moved feature/view raw serde_json \
         sites except genuine protocol-mapper conversions: found {total}, seed \
         {TUI_FEATURE_VIEW_RAW_JSON_SITE_SEED}. Move payload interpretation into a \
         protocol-layer mapper (see quecto-tui/src/protocol/model_payloads.rs, \
         #1220). Inventory (burn-down order): {per_file:?}"
    );
}

#[test]
fn tui_protocol_raw_json_parsing_sites_do_not_grow() {
    let (total, per_file) = tui_ratchet_inventory(
        &[TUI_PROTOCOL],
        TUI_PROTOCOL_RAW_JSON_ALLOWLIST,
        raw_json_site_count,
    );
    assert_eq!(
        total, TUI_PHASE_6_PROTOCOL_RAW_JSON_TOTAL,
        "#1257 Phase 6 protocol mapper raw serde_json sites: found {total}, seed \
         {TUI_PROTOCOL_RAW_JSON_SITE_SEED}. Convert ad-hoc parsing into typed \
         protocol mappers and lower this seed. Inventory (burn-down order): \
         {per_file:?}"
    );
    assert!(
        total <= TUI_PROTOCOL_RAW_JSON_SITE_SEED,
        "raw serde_json parsing in TUI protocol mappers must not grow: \
         found {total}, seed {TUI_PROTOCOL_RAW_JSON_SITE_SEED}. Inventory \
         (burn-down order): {per_file:?}"
    );
}

#[test]
fn tui_combined_raw_json_inventory_does_not_grow() {
    let (feature_total, feature_files) = tui_ratchet_inventory(
        TUI_FEATURE_VIEW_RATCHET_ROOTS,
        TUI_FEATURE_VIEW_RAW_JSON_ALLOWLIST,
        raw_json_site_count,
    );
    let (protocol_total, protocol_files) = tui_ratchet_inventory(
        &[TUI_PROTOCOL],
        TUI_PROTOCOL_RAW_JSON_ALLOWLIST,
        raw_json_site_count,
    );
    let total = feature_total + protocol_total;
    assert!(
        total <= TUI_RAW_JSON_COMBINED_CEILING,
        "combined TUI raw serde_json inventory must not grow when sites move between feature/view and protocol buckets: found {total} ({feature_total} feature/view + {protocol_total} protocol), historical ceiling {TUI_RAW_JSON_COMBINED_CEILING}. Feature inventory: {feature_files:?}; protocol inventory: {protocol_files:?}"
    );
}

#[test]
fn tui_wire_dto_usage_does_not_grow() {
    let (total, per_file) = tui_ratchet_inventory(
        TUI_FEATURE_VIEW_RATCHET_ROOTS,
        TUI_WIRE_DTO_ALLOWLIST,
        wire_dto_usage_count,
    );
    assert_eq!(
        total, TUI_PHASE_6_WIRE_DTO_USAGE_TOTAL,
        "#1257 Phase 6 relocation must preserve TUI feature/view wire-DTO usage: \
         found {total}, seed {TUI_WIRE_DTO_USAGE_SEED} (#1220). Counting usages, not \
         `use` lines, so `use super::*` and fully-qualified paths are visible. \
         Inventory (burn-down order): {per_file:?}"
    );
}

// ── #1257 Phase 6 responsibility-based purity checks ────────────────

#[test]
fn tui_components_do_not_import_protocol_client() {
    // Components are presentation widgets: they must not depend on the UDS
    // client/wire DTOs. Residual raw-JSON in chat_render remains measured by
    // the feature/view ratchet; this check enforces the transport boundary.
    assert_no_imports(
        "quecto-tui components",
        Path::new(TUI_COMPONENTS),
        &["crate::protocol::client"],
    );
}

#[test]
fn tui_protocol_has_no_widget_or_terminal_types() {
    assert_no_imports(
        "quecto-tui protocol",
        Path::new(TUI_PROTOCOL),
        &[
            "crate::components",
            "crate::shell::terminal",
            "crate::shell::render",
            "crate::shell::keys",
            "crate::shell::app",
        ],
    );
}

#[test]
fn tui_shell_owns_app_composition_root() {
    assert!(
        Path::new(TUI_SHELL).join("app.rs").is_file(),
        "shell must own App composition root after #1257 Phase 6"
    );
    assert!(
        Path::new(TUI_SHELL).join("stdin_buffer.rs").is_file(),
        "shell must own stdin_buffer after #1257 Phase 6"
    );
    assert!(
        Path::new(TUI_COMPONENTS).join("ansi.rs").is_file(),
        "components must own ansi after #1257 Phase 6"
    );
    assert!(
        Path::new(TUI_COMPONENTS).join("theme.rs").is_file(),
        "components must own theme after #1257 Phase 6"
    );
    assert!(
        !Path::new("../quecto-tui/src/interface").exists(),
        "interface/ must be fully retired after #1257 Phase 6"
    );
}
