use super::*;

#[then(expr = "the source file {string} should contain {string}")]
fn then_source_file_contains(_world: &mut QuectoWorld, path: String, needle: String) {
    let content = std::fs::read_to_string(&path).expect("read source file");
    assert!(
        content.contains(&needle),
        "expected '{}' to contain '{}', but it did not",
        path,
        needle
    );
}

#[then(expr = "the source file {string} should not contain {string}")]
fn then_source_file_not_contains(_world: &mut QuectoWorld, path: String, needle: String) {
    let content = std::fs::read_to_string(&path).expect("read source file");
    assert!(
        !content.contains(&needle),
        "expected '{}' to not contain '{}', but it did",
        path,
        needle
    );
}

#[then("the application source should not contain runtime I/O patterns")]
fn then_application_has_no_runtime_io(_world: &mut QuectoWorld) {
    let mut files = Vec::new();
    collect_rs_files(Path::new("src/application"), &mut files);

    let forbidden = [
        "std::fs::",
        "tokio::fs::",
        "std::env::",
        "dirs::",
        ".exists(",
    ];

    for file_content in &files {
        let (file_path, _) = file_content
            .split_once(":\n")
            .expect("split path from file content");

        for line in file_content.lines().skip(1) {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") {
                continue;
            }

            for pattern in &forbidden {
                assert!(
                    !trimmed.contains(pattern),
                    "application runtime I/O pattern found in {}: {}",
                    file_path,
                    trimmed
                );
            }
        }
    }
}

fn collect_rs_files(dir: &Path, files: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }

    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = std::fs::read_to_string(&path).expect("read file");
            files.push(format!("{}:\n{}", path.display(), content));
        }
    }
}

#[then("the pre-push script should lint with --workspace flag")]
fn then_pre_push_lints_workspace(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("scripts/pre-push.sh").expect("read scripts/pre-push.sh");
    // Find the actual clippy invocation line (not echo/comment lines).
    let has_workspace_clippy = content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("cargo clippy") && trimmed.contains("--workspace")
    });
    assert!(
        has_workspace_clippy,
        "pre-push.sh must invoke `cargo clippy --workspace` to lint all workspace members including quecto-tui"
    );
}
