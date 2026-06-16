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

#[then("the quecto-tui infrastructure source should not import interface layers")]
fn then_tui_infrastructure_no_interface(_world: &mut QuectoWorld) {
    assert_no_tui_patterns("infrastructure", &["crate::interface", "super::interface"]);
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
