use super::*;
use tempfile::TempDir;

fn setup_repo() -> (TempDir, PathBuf) {
    let td = TempDir::new().unwrap();
    let job_dir = td.path().to_path_buf();
    std::fs::create_dir_all(job_dir.join("src")).unwrap();
    std::fs::write(
        job_dir.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        job_dir.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();
    std::fs::write(job_dir.join(".gitignore"), "target/\n*.log\n").unwrap();
    (td, job_dir)
}

fn ep<'a>(
    job_dir: &'a Path,
    file_path: &'a str,
    old_string: &'a str,
    new_string: &'a str,
) -> EditParams<'a> {
    EditParams {
        job_dir,
        file_path,
        old_string,
        new_string,
        preview_only: false,
        fuzzy: false,
    }
}

#[test]
fn test_edit_exact_replace() {
    let (_td, job_dir) = setup_repo();
    let result = edit_file(&ep(&job_dir, "src/main.rs", "hello", "world"));
    assert!(result.ok);
    assert!(result.diff.is_some());
    assert!(result.first_changed_line.is_some());
    let content = std::fs::read_to_string(job_dir.join("src/main.rs")).unwrap();
    assert!(content.contains("world"));
    assert!(!content.contains("hello"));
}

#[test]
fn test_edit_ambiguous() {
    let (_td, job_dir) = setup_repo();
    let result = edit_file(&ep(&job_dir, "src/lib.rs", "a", "x"));
    assert!(!result.ok);
    assert!(result.error.as_ref().unwrap().contains("ambiguous"));
    assert!(result.match_count.unwrap() > 1);
    assert!(result.match_lines.is_some());
}

#[test]
fn test_edit_noop() {
    let (_td, job_dir) = setup_repo();
    let result = edit_file(&ep(&job_dir, "src/main.rs", "hello", "hello"));
    assert!(!result.ok);
    assert!(result.error.as_ref().unwrap().contains("no-op"));
}

#[test]
fn test_edit_preview() {
    let (_td, job_dir) = setup_repo();
    let before = std::fs::read_to_string(job_dir.join("src/main.rs")).unwrap();
    let result = edit_file(&EditParams {
        preview_only: true,
        ..ep(&job_dir, "src/main.rs", "hello", "world")
    });
    assert!(result.ok);
    assert!(result.diff.is_some());
    let after = std::fs::read_to_string(job_dir.join("src/main.rs")).unwrap();
    assert_eq!(before, after, "file should not change in preview mode");
}

#[test]
fn test_edit_crlf() {
    let (_td, job_dir) = setup_repo();
    std::fs::write(
        job_dir.join("src/main.rs"),
        "fn main() {\r\n    println!(\"hello\");\r\n}\r\n",
    )
    .unwrap();
    let result = edit_file(&ep(&job_dir, "src/main.rs", "hello", "world"));
    assert!(result.ok);
    let content = std::fs::read_to_string(job_dir.join("src/main.rs")).unwrap();
    assert!(content.contains("\r\n"), "should retain CRLF");
    assert!(content.contains("world"));
}

#[test]
fn test_edit_bom() {
    let (_td, job_dir) = setup_repo();
    std::fs::write(
        job_dir.join("src/main.rs"),
        "\u{feff}fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    let result = edit_file(&ep(&job_dir, "src/main.rs", "hello", "world"));
    assert!(result.ok);
    let content = std::fs::read_to_string(job_dir.join("src/main.rs")).unwrap();
    assert!(content.starts_with('\u{feff}'), "BOM should be preserved");
}

#[test]
fn test_edit_fuzzy() {
    let (_td, job_dir) = setup_repo();
    std::fs::write(
        job_dir.join("src/main.rs"),
        "fn main() {  \n    println!(\"hello\");  \n}\n",
    )
    .unwrap();
    let result = edit_file(&EditParams {
        fuzzy: true,
        ..ep(&job_dir, "src/main.rs", "hello", "world")
    });
    assert!(result.ok, "fuzzy edit should succeed");
}

#[test]
fn test_is_destructive_git() {
    assert!(is_destructive_git_command("git push --force"));
    assert!(is_destructive_git_command("git reset --hard HEAD~5"));
    assert!(is_destructive_git_command("git clean -fd"));
    assert!(!is_destructive_git_command("git status"));
    assert!(!is_destructive_git_command("git diff"));
    assert!(!is_destructive_git_command("git add ."));
    assert!(!is_destructive_git_command("git commit -m 'msg'"));
}

#[test]
fn test_path_boundary() {
    let (_td, job_dir) = setup_repo();
    assert!(is_within_job_dir(&job_dir.join("src/main.rs"), &job_dir));
    assert!(!is_within_job_dir(Path::new("/etc/passwd"), &job_dir));
    assert!(!is_within_job_dir(Path::new("/tmp/escape.txt"), &job_dir));
}

#[test]
fn test_grep_content() {
    let (_td, job_dir) = setup_repo();
    let result = grep_content(&job_dir, "fn ", true);
    assert!(result.ok);
    assert!(result.matches.len() >= 2);
    assert!(result.matches.iter().any(|m| m.file.contains("main.rs")));
    assert!(result.matches.iter().any(|m| m.file.contains("lib.rs")));
}

#[test]
fn test_grep_no_matches() {
    let (_td, job_dir) = setup_repo();
    let result = grep_content(&job_dir, "nonexistent_xyz", true);
    assert!(result.ok);
    assert!(result.matches.is_empty());
}

#[test]
fn test_grep_respects_gitignore() {
    let (_td, job_dir) = setup_repo();
    std::fs::create_dir_all(job_dir.join("target/debug")).unwrap();
    std::fs::write(job_dir.join("target/debug/output.log"), "fn test").unwrap();
    let result = grep_content(&job_dir, "fn", true);
    assert!(result.ok);
    assert!(
        !result.matches.iter().any(|m| m.file.starts_with("target/")),
        "gitignored files should be excluded"
    );
}

#[test]
fn test_find_files_glob() {
    let (_td, job_dir) = setup_repo();
    let result = find_files(&job_dir, "src/**/*.rs", true);
    assert!(result.ok);
    assert!(result.files.iter().any(|f| f.contains("main.rs")));
    assert!(result.files.iter().any(|f| f.contains("lib.rs")));
}

#[test]
fn test_find_sorted() {
    let (_td, job_dir) = setup_repo();
    let result = find_files(&job_dir, "**/*.rs", true);
    assert!(result.ok);
    let mut sorted = result.files.clone();
    sorted.sort();
    assert_eq!(result.files, sorted, "results should be sorted");
}

#[test]
fn test_read_paginated() {
    let (_td, job_dir) = setup_repo();
    let result = read_file_paginated(&job_dir, "src/main.rs", 1, 1);
    assert!(result.ok);
    assert_eq!(result.offset, 1);
    assert!(result.has_more);
}

#[test]
fn test_read_path_violation() {
    let (_td, job_dir) = setup_repo();
    let result = read_file_paginated(&job_dir, "/etc/passwd", 0, 100);
    assert!(!result.ok);
    assert!(result.error.as_ref().unwrap().contains("path violation"));
}

#[test]
fn test_smart_punctuation() {
    let (_td, job_dir) = setup_repo();
    std::fs::write(
        job_dir.join("README.md"),
        "This is a \u{201C}smart quote\u{201D} test\n",
    )
    .unwrap();
    let result = edit_file(&ep(&job_dir, "README.md", "\"smart quote\"", "\"fixed\""));
    assert!(result.ok, "smart punctuation normalization should work");
}
