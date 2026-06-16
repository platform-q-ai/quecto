use super::*;

const TUI_ROOT: &str = "quecto-tui/src";

#[then(expr = "the quecto-tui source tree should contain layer {string}")]
fn then_tui_source_tree_contains_layer(_world: &mut QuectoWorld, layer: String) {
    let path = Path::new(TUI_ROOT).join(&layer);
    assert!(
        path.is_dir(),
        "quecto-tui layer directory must exist: {}",
        path.display()
    );
    assert!(
        path.join("mod.rs").is_file(),
        "quecto-tui layer must expose mod.rs: {}",
        path.display()
    );
}

#[then("the quecto-tui domain source should not contain runtime I/O patterns")]
fn then_tui_domain_no_runtime_io(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "domain",
        &[
            "std::fs::",
            "tokio::fs::",
            "std::env::",
            "dirs::",
            ".exists(",
        ],
    );
}

#[then("the quecto-tui application source should not contain runtime I/O patterns")]
fn then_tui_application_no_runtime_io(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "application",
        &[
            "std::fs::",
            "tokio::fs::",
            "std::env::",
            "dirs::",
            ".exists(",
        ],
    );
}

#[then("the quecto-tui domain source should not import outer layers")]
fn then_tui_domain_no_outer_layers(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "domain",
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

#[then("the quecto-tui application source should not import infrastructure or interface layers")]
fn then_tui_application_imports_only_inward(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "application",
        &[
            "crate::infrastructure",
            "crate::interface",
            "super::infrastructure",
            "super::interface",
        ],
    );
}

#[then("the quecto-tui infrastructure source should not import application or interface layers")]
fn then_tui_infrastructure_no_application_or_interface(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "infrastructure",
        &[
            "crate::application",
            "crate::interface",
            "super::application",
            "super::interface",
        ],
    );
}

#[then("every quecto-tui production Rust file should be under a Clean Architecture layer")]
fn then_every_tui_production_file_is_layered(_world: &mut QuectoWorld) {
    let misplaced = misplaced_tui_production_files();
    assert!(
        misplaced.is_empty(),
        "quecto-tui production Rust files must live under domain/, application/, infrastructure/, or interface/; misplaced: {misplaced:?}"
    );
}

#[then("the quecto-tui library root should expose only Clean Architecture layers")]
fn then_tui_library_root_exposes_only_layers(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("quecto-tui/src/lib.rs").expect("read quecto-tui lib.rs");
    let public_modules: Vec<_> = content
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("pub mod "))
        .map(|rest| rest.trim_end_matches(';'))
        .collect();
    assert_eq!(
        public_modules,
        ["application", "domain", "infrastructure", "interface"],
        "quecto-tui/src/lib.rs should match the main crate shape and expose only architecture layers"
    );
    assert!(
        !content.contains("#[path ="),
        "quecto-tui/src/lib.rs should not re-export interface internals with #[path] shims"
    );
}

#[then("the quecto-tui binary root should delegate to the interface layer")]
fn then_tui_binary_root_delegates_to_interface(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("quecto-tui/src/main.rs").expect("read quecto-tui main.rs");
    assert!(
        content.contains("quecto_tui::interface::cli") && content.lines().count() <= 10,
        "quecto-tui/src/main.rs should be a thin binary entrypoint delegating to interface::cli"
    );
}

#[then("the architecture test target should enforce quecto-tui Clean Architecture layers")]
fn then_architecture_test_enforces_tui_layers(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("tests/architecture.rs").expect("read architecture tests");
    assert!(
        content.contains("fn tui_architecture_layers_exist")
            && content.contains("fn tui_domain_has_no_outer_layer_imports")
            && content.contains("fn tui_application_has_no_infrastructure_or_interface_imports")
            && content.contains("fn tui_infrastructure_has_no_application_or_interface_imports"),
        "tests/architecture.rs must enforce quecto-tui layer existence and dependency direction"
    );
}

#[then("the architecture test target should enforce quecto-tui runtime I/O boundaries")]
fn then_architecture_test_enforces_tui_runtime_io(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("tests/architecture.rs").expect("read architecture tests");
    assert!(
        content.contains("fn tui_inner_layers_have_no_runtime_io_calls")
            && content.contains("quecto-tui domain")
            && content.contains("quecto-tui application"),
        "tests/architecture.rs must enforce runtime I/O boundaries for quecto-tui inner layers"
    );
}

#[then("the architecture test target should enforce quecto-tui root file placement")]
fn then_architecture_test_enforces_tui_root_file_placement(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("tests/architecture.rs").expect("read architecture tests");
    assert!(
        content.contains("fn tui_production_files_live_inside_architecture_layers")
            && content.contains("TUI_ALLOWED_ROOT_RS")
            && content.contains("fn tui_lib_rs_exposes_only_architecture_layers")
            && content.contains("fn tui_main_rs_is_thin_interface_entrypoint"),
        "tests/architecture.rs must reject unlayered quecto-tui production source files and keep crate roots thin"
    );
}

fn misplaced_tui_production_files() -> Vec<String> {
    let mut misplaced = Vec::new();
    collect_misplaced_tui_rs_files(Path::new(TUI_ROOT), &mut misplaced);
    misplaced
}

fn collect_misplaced_tui_rs_files(dir: &Path, misplaced: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read quecto-tui src dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_misplaced_tui_rs_files(&path, misplaced);
            continue;
        }
        if !path.extension().is_some_and(|ext| ext == "rs") {
            continue;
        }
        let rel = path
            .strip_prefix(TUI_ROOT)
            .expect("strip quecto-tui src prefix")
            .to_string_lossy()
            .replace('\\', "/");
        let top = rel.split('/').next().unwrap_or_default();
        let in_layer = matches!(
            top,
            "domain" | "application" | "infrastructure" | "interface"
        );
        let allowed_root = !rel.contains('/') && matches!(rel.as_str(), "lib.rs" | "main.rs");
        if !in_layer && !allowed_root {
            misplaced.push(rel);
        }
    }
}

#[then("the BDD runner should execute TUI scenarios tagged wip or done")]
fn then_bdd_runner_executes_tui_wip_or_done(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("tests/bdd/main.rs").expect("read BDD runner");
    assert!(
        content.contains("tag_filter") && content.contains("wip") && content.contains("done"),
        "BDD runner must support executing selected @tui scenarios when they are tagged @wip or @done"
    );
}

#[then("the TUI architecture feature should not contain pending scenarios")]
fn then_tui_architecture_feature_not_pending(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("tests/features/tui_clean_architecture.feature")
        .expect("read TUI architecture feature");
    assert!(
        !content.contains("@pending"),
        "TUI architecture feature must remain executable and not be marked @pending"
    );
}

fn assert_no_tui_patterns(layer: &str, forbidden: &[&str]) {
    let dir = Path::new(TUI_ROOT).join(layer);
    let mut files = Vec::new();
    collect_tui_rs_files(&dir, &mut files);
    assert!(
        !files.is_empty(),
        "quecto-tui {layer} layer must contain Rust source files"
    );

    for file_content in &files {
        let (file_path, _) = file_content
            .split_once(":\n")
            .expect("split path from file content");
        for line in file_content.lines().skip(1) {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") || trimmed.starts_with("//!") {
                continue;
            }
            for pattern in forbidden {
                assert!(
                    !trimmed.contains(pattern),
                    "quecto-tui {layer} architecture violation in {file_path}: {trimmed}; forbidden pattern: {pattern}"
                );
            }
        }
    }
}

fn collect_tui_rs_files(dir: &Path, files: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read TUI layer dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_tui_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read TUI source file");
            files.push(format!("{}:\n{}", path.display(), content));
        }
    }
}
