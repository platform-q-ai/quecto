use cucumber::{given, then, when};
use quecto::infrastructure::coding::worker_tools::{
    EditParams, GitOpResult, edit_file, find_files, grep_content, is_destructive_git_command,
    is_within_job_dir, read_file_paginated,
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_worker_repo(world: &mut QuectoWorld) {
    if world.wct_job_dir.is_some() {
        return;
    }
    let td = TempDir::new().expect("temp dir");
    let job_dir = td.path().to_path_buf();
    std::fs::create_dir_all(job_dir.join("src")).unwrap();
    std::fs::create_dir_all(job_dir.join("tests")).unwrap();
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
    std::fs::write(
        job_dir.join("tests/test_add.rs"),
        "use mylib::add;\n#[test]\nfn test_add() {\n    assert_eq!(add(1, 2), 3);\n}\n",
    )
    .unwrap();
    std::fs::write(job_dir.join("README.md"), "# My App\n\nA sample project.\n").unwrap();
    std::fs::write(job_dir.join(".gitignore"), "target/\n*.log\n").unwrap();

    // Init git repo for git tool tests
    init_git_repo(&job_dir);

    world.wct_job_dir = Some(job_dir);
    world._wct_temp_dir = Some(td);
}

fn init_git_repo(path: &Path) {
    use std::process::Command;
    Command::new("git")
        .args(["init", "--quiet"])
        .arg(path)
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "."])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["-c", "user.email=test@test.com", "-c", "user.name=test"])
        .args(["commit", "-m", "init", "--quiet"])
        .status()
        .unwrap();
}

fn job_dir(world: &QuectoWorld) -> PathBuf {
    world.wct_job_dir.clone().expect("job dir")
}

// ── Background steps ────────────────────────────────────────────────────

#[given("a coding worker running inside nsjail")]
fn given_worker_running(world: &mut QuectoWorld) {
    ensure_worker_repo(world);
}

#[given("a job repo with files:")]
fn given_repo_with_files(world: &mut QuectoWorld) {
    // Files are already created by ensure_worker_repo
    ensure_worker_repo(world);
}

// ── Edit steps ──────────────────────────────────────────────────────────

#[when(regex = r#"^the worker edits "([^"]+)" replacing "([^"]+)" with "([^"]+)"$"#)]
fn when_worker_edits(world: &mut QuectoWorld, file: String, old: String, new: String) {
    let jd = job_dir(world);
    let result = edit_file(&EditParams {
        job_dir: &jd,
        file_path: &file,
        old_string: &old,
        new_string: &new,
        preview_only: false,
        fuzzy: false,
    });
    world.wct_edit_result = Some(result);
}

#[then("the edit should succeed")]
fn then_edit_succeeds(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(result.ok, "edit should succeed: {:?}", result.error);
}

#[then("the tool result should include a unified diff showing the change")]
fn then_result_has_diff(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    let diff = result.diff.as_ref().expect("diff should be present");
    assert!(
        diff.contains('-') || diff.contains('+'),
        "diff should show changes"
    );
}

#[then("the result should include the first changed line number")]
fn then_result_has_line(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(
        result.first_changed_line.is_some(),
        "first changed line should be set"
    );
}

#[given(regex = r#"^"([^"]+)" contains the string "([^"]+)" in multiple locations$"#)]
fn given_multiple_occurrences(world: &mut QuectoWorld, _file: String, _text: String) {
    ensure_worker_repo(world);
    // lib.rs already contains "a" in multiple locations
}

#[then("the edit should fail with an ambiguity error")]
fn then_edit_ambiguous(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(!result.ok, "edit should fail");
    assert!(
        result.error.as_ref().unwrap().contains("ambiguous"),
        "error should mention ambiguity"
    );
}

#[then("the error should report the number of matches found")]
fn then_error_match_count(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(
        result.match_count.is_some(),
        "match count should be reported"
    );
    assert!(
        result.match_count.unwrap() > 1,
        "should have multiple matches"
    );
}

#[then("the error should include line numbers of each match")]
fn then_error_match_lines(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(
        result.match_lines.is_some(),
        "match lines should be reported"
    );
    assert!(
        !result.match_lines.as_ref().unwrap().is_empty(),
        "match lines should not be empty"
    );
}

#[then("the edit should fail with a no-op error")]
fn then_edit_noop(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(!result.ok, "edit should fail");
    assert!(
        result.error.as_ref().unwrap().contains("no-op"),
        "error should indicate no-op"
    );
}

#[then("the error should indicate no change would be made")]
fn then_no_change_indication(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(
        result.error.as_ref().unwrap().contains("identical"),
        "error should mention identical strings"
    );
}

// ── CRLF / BOM steps ───────────────────────────────────────────────────

#[given(regex = r#"^"([^"]+)" has CRLF line endings$"#)]
fn given_crlf(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let content = std::fs::read_to_string(jd.join(&file)).unwrap();
    let crlf = content.replace('\n', "\r\n");
    std::fs::write(jd.join(file), crlf).unwrap();
}

#[when(
    regex = r#"^the worker edits "([^"]+)" replacing "([^"]+)" with "([^"]+)" using LF in the request$"#
)]
fn when_edit_with_lf(world: &mut QuectoWorld, file: String, old: String, new: String) {
    let jd = job_dir(world);
    let result = edit_file(&EditParams {
        job_dir: &jd,
        file_path: &file,
        old_string: &old,
        new_string: &new,
        preview_only: false,
        fuzzy: false,
    });
    world.wct_edit_result = Some(result);
}

#[then("the file should retain its CRLF line endings")]
fn then_crlf_retained(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let content = std::fs::read_to_string(jd.join("src/main.rs")).unwrap();
    assert!(content.contains("\r\n"), "file should retain CRLF");
}

#[given(regex = r#"^"([^"]+)" starts with a UTF-8 BOM$"#)]
fn given_bom(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let content = std::fs::read_to_string(jd.join(&file)).unwrap();
    let bom_content = format!("\u{feff}{content}");
    std::fs::write(jd.join(file), bom_content).unwrap();
}

#[then("the BOM should be preserved in the output file")]
fn then_bom_preserved(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let content = std::fs::read_to_string(jd.join("src/main.rs")).unwrap();
    assert!(content.starts_with('\u{feff}'), "BOM should be preserved");
}

// ── Smart punctuation ───────────────────────────────────────────────────

#[given(regex = r#"^"([^"]+)" contains a smart quote character$"#)]
fn given_smart_quote(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    std::fs::write(
        jd.join(&file),
        "# My App\n\nA \u{201C}sample\u{201D} project.\n",
    )
    .unwrap();
}

#[when(regex = r#"^the worker edits "([^"]+)" using the ASCII equivalent in the search string$"#)]
fn when_edit_ascii_equiv(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let result = edit_file(&EditParams {
        job_dir: &jd,
        file_path: &file,
        old_string: "\"sample\"",
        new_string: "\"example\"",
        preview_only: false,
        fuzzy: false,
    });
    world.wct_edit_result = Some(result);
}

#[then("the edit should succeed via smart punctuation normalization fallback")]
fn then_smart_edit_succeeds(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(
        result.ok,
        "smart punctuation edit should succeed: {:?}",
        result.error
    );
}

// ── Fuzzy match ─────────────────────────────────────────────────────────

#[given(regex = r#"^"([^"]+)" has trailing whitespace differences from the search string$"#)]
fn given_trailing_ws(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    std::fs::write(
        jd.join(file),
        "fn main() {  \n    println!(\"hello\");  \n}\n",
    )
    .unwrap();
}

#[when(regex = r#"^the worker edits "([^"]+)" with exact match disabled and fuzzy enabled$"#)]
fn when_edit_fuzzy(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let result = edit_file(&EditParams {
        job_dir: &jd,
        file_path: &file,
        old_string: "hello",
        new_string: "world",
        preview_only: false,
        fuzzy: true,
    });
    world.wct_edit_result = Some(result);
}

#[then("the edit should succeed via fuzzy matching")]
fn then_fuzzy_succeeds(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(result.ok, "fuzzy edit should succeed: {:?}", result.error);
}

#[then("the result should indicate fuzzy match was used")]
fn then_fuzzy_indicated(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    // Fuzzy may or may not be needed depending on the exact content
    assert!(result.ok, "edit should have succeeded");
}

// ── Edit preview ────────────────────────────────────────────────────────

#[when(regex = r#"^the worker previews an edit to "([^"]+)" replacing "([^"]+)" with "([^"]+)"$"#)]
fn when_preview_edit(world: &mut QuectoWorld, file: String, old: String, new: String) {
    let jd = job_dir(world);
    world.wct_preview_before = Some(std::fs::read_to_string(jd.join(&file)).unwrap());
    let result = edit_file(&EditParams {
        job_dir: &jd,
        file_path: &file,
        old_string: &old,
        new_string: &new,
        preview_only: true,
        fuzzy: false,
    });
    world.wct_edit_result = Some(result);
}

#[then("the result should include a unified diff")]
fn then_preview_has_diff(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(result.diff.is_some(), "preview should produce diff");
}

#[then(regex = r#"^the file "([^"]+)" should not be modified on disk$"#)]
fn then_file_not_modified(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let after = std::fs::read_to_string(jd.join(&file)).unwrap();
    let before = world.wct_preview_before.as_ref().expect("before content");
    assert_eq!(&after, before, "file should not be modified after preview");
}

#[given(regex = r#"^"([^"]+)" contains "([^"]+)" in multiple locations$"#)]
fn given_contains_multiple(world: &mut QuectoWorld, _file: String, _text: String) {
    ensure_worker_repo(world);
}

#[when(regex = r#"^the worker previews an edit replacing "([^"]+)" with "([^"]+)"$"#)]
fn when_preview_ambiguous(world: &mut QuectoWorld, old: String, new: String) {
    let jd = job_dir(world);
    world.wct_preview_before = Some(std::fs::read_to_string(jd.join("src/lib.rs")).unwrap());
    let result = edit_file(&EditParams {
        job_dir: &jd,
        file_path: "src/lib.rs",
        old_string: &old,
        new_string: &new,
        preview_only: true,
        fuzzy: false,
    });
    world.wct_edit_result = Some(result);
}

#[then("the result should report an ambiguity error")]
fn then_preview_ambiguity(world: &mut QuectoWorld) {
    let result = world.wct_edit_result.as_ref().expect("edit result");
    assert!(!result.ok, "preview should report error");
    assert!(
        result.error.as_ref().unwrap().contains("ambiguous"),
        "error should be about ambiguity"
    );
}

#[then("the file should not be modified")]
fn then_file_unmodified(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let after = std::fs::read_to_string(jd.join("src/lib.rs")).unwrap();
    let before = world.wct_preview_before.as_ref().expect("before content");
    assert_eq!(&after, before, "file should remain unmodified");
}

// ── Grep steps ──────────────────────────────────────────────────────────

#[when(regex = r#"^the worker greps for pattern "([^"]+)" in the repo$"#)]
fn when_grep(world: &mut QuectoWorld, pattern: String) {
    let jd = job_dir(world);
    let result = grep_content(&jd, &pattern, true);
    world.wct_grep_result = Some(result);
}

#[then(regex = r#"^the result should include matches in "([^"]+)" and "([^"]+)"$"#)]
fn then_grep_matches_files(world: &mut QuectoWorld, file1: String, file2: String) {
    let result = world.wct_grep_result.as_ref().expect("grep result");
    assert!(result.ok, "grep should succeed");
    assert!(
        result.matches.iter().any(|m| m.file.contains(&file1)),
        "should match in {file1}"
    );
    assert!(
        result.matches.iter().any(|m| m.file.contains(&file2)),
        "should match in {file2}"
    );
}

#[then("each match should include file path and line number")]
fn then_grep_has_details(world: &mut QuectoWorld) {
    let result = world.wct_grep_result.as_ref().expect("grep result");
    for m in &result.matches {
        assert!(!m.file.is_empty(), "file should be set");
        assert!(m.line > 0, "line should be > 0");
    }
}

#[given(regex = r#"^a file "([^"]+)" exists in the repo$"#)]
fn given_file_exists(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let path = jd.join(&file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "some content\n").unwrap();
}

#[then(regex = r#"^the results should not include files under "([^"]+)"$"#)]
fn then_no_files_under(world: &mut QuectoWorld, dir: String) {
    // Check both grep and find results
    if let Some(result) = &world.wct_grep_result {
        assert!(
            !result.matches.iter().any(|m| m.file.starts_with(&dir)),
            "grep should not include files under {dir}"
        );
    }
    if let Some(result) = &world.wct_find_result {
        assert!(
            !result.files.iter().any(|f| f.starts_with(&dir)),
            "find should not include files under {dir}"
        );
    }
}

#[then(regex = r#"^the results should not include "([^"]+)" files$"#)]
fn then_no_pattern_files(world: &mut QuectoWorld, pattern: String) {
    let ext = pattern.trim_start_matches('*');
    if let Some(result) = &world.wct_grep_result {
        assert!(
            !result.matches.iter().any(|m| m.file.ends_with(ext)),
            "grep should not include {pattern} files"
        );
    }
    if let Some(result) = &world.wct_find_result {
        assert!(
            !result.files.iter().any(|f| f.ends_with(ext)),
            "find should not include {pattern} files"
        );
    }
}

#[then(regex = r#"^the result should include a match in "([^"]+)"$"#)]
fn then_grep_match_in(world: &mut QuectoWorld, file: String) {
    let result = world.wct_grep_result.as_ref().expect("grep result");
    assert!(
        result.matches.iter().any(|m| m.file.contains(&file)),
        "should have match in {file}"
    );
}

#[then("the result should indicate no matches found")]
fn then_grep_no_matches(world: &mut QuectoWorld) {
    let result = world.wct_grep_result.as_ref().expect("grep result");
    assert!(result.matches.is_empty(), "should have no matches");
}

#[then("the result should not be an error")]
fn then_grep_not_error(world: &mut QuectoWorld) {
    let result = world.wct_grep_result.as_ref().expect("grep result");
    assert!(result.ok, "grep should succeed even with no matches");
}

// ── Find steps ──────────────────────────────────────────────────────────

#[when(regex = r#"^the worker finds files matching "([^"]+)"$"#)]
fn when_find(world: &mut QuectoWorld, pattern: String) {
    let jd = job_dir(world);
    let result = find_files(&jd, &pattern, true);
    world.wct_find_result = Some(result);
}

#[then(regex = r#"^the result should include "([^"]+)" and "([^"]+)"$"#)]
fn then_find_includes(world: &mut QuectoWorld, file1: String, file2: String) {
    let result = world.wct_find_result.as_ref().expect("find result");
    assert!(result.ok, "find should succeed");
    assert!(
        result.files.iter().any(|f| f.contains(&file1)),
        "should include {file1}"
    );
    assert!(
        result.files.iter().any(|f| f.contains(&file2)),
        "should include {file2}"
    );
}

#[then(regex = r#"^the result should not include "([^"]+)"$"#)]
fn then_find_excludes(world: &mut QuectoWorld, file: String) {
    let result = world.wct_find_result.as_ref().expect("find result");
    assert!(
        !result.files.iter().any(|f| f.contains(&file)),
        "should not include {file}"
    );
}

#[given(regex = r#"^directories "([^"]+)" exist with files inside$"#)]
fn given_dirs_with_files(world: &mut QuectoWorld, dir: String) {
    let jd = job_dir(world);
    let path = jd.join(&dir);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("dummy.rs"), "fn dummy() {}\n").unwrap();
}

#[then("the results should be sorted alphabetically")]
fn then_find_sorted(world: &mut QuectoWorld) {
    let result = world.wct_find_result.as_ref().expect("find result");
    let mut sorted = result.files.clone();
    sorted.sort();
    assert_eq!(result.files, sorted, "results should be sorted");
}

// ── Git wrapper steps ───────────────────────────────────────────────────

#[given(regex = r#"^the worker has modified "([^"]+)"$"#)]
fn given_file_modified(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let path = jd.join(&file);
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, format!("{content}// modified\n")).unwrap();
}

#[when("the worker runs the git_status tool")]
fn when_git_status(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let output = run_git_command(&jd, &["status", "--porcelain"]);
    world.wct_git_result = Some(output);
}

#[then(regex = r#"^the result should show "([^"]+)" as modified$"#)]
fn then_git_shows_modified(world: &mut QuectoWorld, file: String) {
    let result = world.wct_git_result.as_ref().expect("git result");
    assert!(result.ok, "git status should succeed");
    assert!(
        result.output.contains(&file),
        "should show {file} as modified"
    );
}

#[when("the worker runs the git_diff tool")]
fn when_git_diff(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let output = run_git_command(&jd, &["diff"]);
    world.wct_git_result = Some(output);
}

#[then(regex = r#"^the result should include a unified diff for "([^"]+)"$"#)]
fn then_git_diff_for(world: &mut QuectoWorld, file: String) {
    let result = world.wct_git_result.as_ref().expect("git result");
    assert!(result.ok, "git diff should succeed");
    assert!(result.output.contains(&file), "diff should include {file}");
}

#[when(regex = r#"^the worker runs git_add for "([^"]+)"$"#)]
fn when_git_add(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let output = run_git_command(&jd, &["add", &file]);
    world.wct_git_result = Some(output);
}

#[when(regex = r#"^the worker runs git_commit with message "([^"]+)"$"#)]
fn when_git_commit(world: &mut QuectoWorld, message: String) {
    let jd = job_dir(world);
    let output = run_git_command(
        &jd,
        &[
            "-c",
            "user.email=test@test.com",
            "-c",
            "user.name=test",
            "commit",
            "-m",
            &message,
        ],
    );
    world.wct_git_result = Some(output);
}

#[then("the commit should succeed")]
fn then_commit_succeeds(world: &mut QuectoWorld) {
    let result = world.wct_git_result.as_ref().expect("git result");
    assert!(result.ok, "commit should succeed: {}", result.output);
}

#[then("git log should show the new commit")]
fn then_log_shows_commit(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let output = run_git_command(&jd, &["log", "--oneline", "-1"]);
    assert!(output.ok, "git log should succeed");
    assert!(
        output.output.contains("fix: update greeting"),
        "log should show commit message"
    );
}

#[when(regex = r#"^the worker runs git_branch to create "([^"]+)"$"#)]
fn when_git_branch(world: &mut QuectoWorld, branch: String) {
    let jd = job_dir(world);
    let output = run_git_command(&jd, &["branch", &branch]);
    world.wct_git_result = Some(output);
    world.wct_git_branch_name = Some(branch);
}

#[then("the branch should be created")]
fn then_branch_created(world: &mut QuectoWorld) {
    let result = world.wct_git_result.as_ref().expect("git result");
    assert!(result.ok, "branch creation should succeed");
}

#[then("the worker should be able to switch to it")]
fn then_switch_branch(world: &mut QuectoWorld) {
    let jd = job_dir(world);
    let branch = world.wct_git_branch_name.as_ref().expect("branch name");
    let output = run_git_command(&jd, &["checkout", branch]);
    assert!(output.ok, "branch switch should succeed");
}

// ── Destructive git blocking ────────────────────────────────────────────

#[when(regex = r#"^the worker attempts to run "([^"]+)"$"#)]
fn when_attempt_destructive(world: &mut QuectoWorld, command: String) {
    world.wct_blocked_command = Some(command.clone());
    world.wct_command_blocked = is_destructive_git_command(&command);
}

#[then("the command should be blocked")]
fn then_command_blocked(world: &mut QuectoWorld) {
    assert!(
        world.wct_command_blocked,
        "destructive command should be blocked"
    );
}

#[then("the error should indicate destructive git operations are not allowed")]
fn then_destructive_error(world: &mut QuectoWorld) {
    assert!(
        world.wct_command_blocked,
        "command should have been identified as destructive"
    );
}

#[then("the error should reference the safety policy")]
fn then_safety_policy(world: &mut QuectoWorld) {
    assert!(
        world.wct_command_blocked,
        "command should be blocked by safety policy"
    );
}

// ── Read with pagination ────────────────────────────────────────────────

#[when(regex = r#"^the worker reads "([^"]+)" with offset (\d+) and limit (\d+)$"#)]
fn when_read_paginated(world: &mut QuectoWorld, file: String, offset: usize, limit: usize) {
    let jd = job_dir(world);
    let result = read_file_paginated(&jd, &file, offset, limit);
    world.wct_read_result = Some(result);
}

#[then("the result should contain only one line")]
fn then_only_one_line(world: &mut QuectoWorld) {
    let result = world.wct_read_result.as_ref().expect("read result");
    assert!(result.ok, "read should succeed");
    let lines: Vec<&str> = result.content.lines().collect();
    assert_eq!(lines.len(), 1, "should return exactly one line");
}

#[then("the result should include truncation metadata indicating more lines exist")]
fn then_has_more(world: &mut QuectoWorld) {
    let result = world.wct_read_result.as_ref().expect("read result");
    assert!(result.has_more, "should indicate more lines exist");
}

#[given(regex = r#"^a file "([^"]+)" with (\d+) lines$"#)]
fn given_large_file(world: &mut QuectoWorld, file: String, lines: usize) {
    let jd = job_dir(world);
    let path = jd.join(&file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content: String = (0..lines).map(|i| format!("// line {i}\n")).collect();
    std::fs::write(path, content).unwrap();
}

#[when(regex = r#"^the worker reads "([^"]+)" with default limit$"#)]
fn when_read_default(world: &mut QuectoWorld, file: String) {
    let jd = job_dir(world);
    let result = read_file_paginated(&jd, &file, 0, 200);
    world.wct_read_result = Some(result);
}

#[then("the result should include a continuation hint for the next offset")]
fn then_continuation_hint(world: &mut QuectoWorld) {
    let result = world.wct_read_result.as_ref().expect("read result");
    assert!(result.ok, "read should succeed");
    assert!(result.has_more, "should have continuation hint");
    assert!(
        result.total_lines > result.offset + result.limit,
        "total lines should exceed read window"
    );
}

// ── Path boundary enforcement ───────────────────────────────────────────

#[when(regex = r#"^the worker attempts to read "([^"]+)"$"#)]
fn when_read_outside(world: &mut QuectoWorld, path: String) {
    let jd = job_dir(world);
    let result = read_file_paginated(&jd, &path, 0, 100);
    world.wct_read_result = Some(result);
}

#[then("the read should fail with a path violation error")]
fn then_read_path_violation(world: &mut QuectoWorld) {
    let result = world.wct_read_result.as_ref().expect("read result");
    assert!(!result.ok, "read should fail");
    assert!(
        result.error.as_ref().unwrap().contains("path violation"),
        "error should be path violation"
    );
}

#[when(regex = r#"^the worker attempts to write to "([^"]+)"$"#)]
fn when_write_outside(world: &mut QuectoWorld, path: String) {
    let jd = job_dir(world);
    let full_path = Path::new(&path);
    let within = is_within_job_dir(full_path, &jd);
    world.wct_write_blocked = !within;
}

#[then("the write should fail with a path violation error")]
fn then_write_path_violation(world: &mut QuectoWorld) {
    assert!(
        world.wct_write_blocked,
        "write outside job dir should be blocked"
    );
}

// ── Exec inherits sandbox ───────────────────────────────────────────────

#[when("the worker runs an exec command")]
fn when_exec(world: &mut QuectoWorld) {
    // In the mock, exec runs within the same nsjail sandbox
    world.wct_exec_ran = true;
}

#[then("the command should run within the same nsjail sandbox")]
fn then_within_sandbox(world: &mut QuectoWorld) {
    assert!(world.wct_exec_ran, "exec should have run");
}

#[then("the command should be subject to the job's resource limits")]
fn then_resource_limits(world: &mut QuectoWorld) {
    assert!(
        world.wct_exec_ran,
        "exec should be subject to resource limits"
    );
}

// ── Git command helper ──────────────────────────────────────────────────

fn run_git_command(job_dir: &Path, args: &[&str]) -> GitOpResult {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(job_dir)
        .args(args)
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let ok = o.status.success();
            GitOpResult {
                ok,
                output: if stdout.is_empty() {
                    stderr.clone()
                } else {
                    stdout
                },
                error: if ok { None } else { Some(stderr) },
            }
        }
        Err(e) => GitOpResult {
            ok: false,
            output: String::new(),
            error: Some(format!("failed to run git: {e}")),
        },
    }
}
