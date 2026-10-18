use super::*;

#[test]
fn test_match_simple_git_commit() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns("git commit -m wip", &patterns));
}

#[test]
fn test_match_git_push() {
    let patterns = vec!["git push".into()];
    assert!(command_matches_patterns("git push origin main", &patterns));
}

#[test]
fn test_match_gh_pr_merge() {
    let patterns = vec!["gh pr merge".into()];
    assert!(command_matches_patterns(
        "gh pr merge 42 --squash",
        &patterns
    ));
}

#[test]
fn test_no_match_git_add() {
    let patterns = vec!["git commit".into(), "git push".into()];
    assert!(!command_matches_patterns("git add .", &patterns));
}

#[test]
fn test_no_match_git_status() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("git status", &patterns));
}

#[test]
fn test_no_match_git_log() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("git log --oneline", &patterns));
}

#[test]
fn test_no_match_git_diff() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("git diff", &patterns));
}

#[test]
fn test_match_git_merge() {
    let patterns = vec!["git merge".into()];
    assert!(command_matches_patterns(
        "git merge --squash feature",
        &patterns
    ));
}

#[test]
fn test_no_match_empty_patterns() {
    let patterns: Vec<String> = vec![];
    assert!(!command_matches_patterns("git commit -m wip", &patterns));
}

#[test]
fn test_no_match_empty_command() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("", &patterns));
}

#[test]
fn test_match_with_flags_before_subcommand() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "git -c user.name=test commit -m wip",
        &patterns
    ));
}

#[test]
fn test_match_with_git_dir_flag() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "git --git-dir /tmp/repo commit -m x",
        &patterns
    ));
}

#[test]
fn test_match_with_capital_c_flag() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "git -C /path commit -m x",
        &patterns
    ));
}

#[test]
fn test_match_chained_with_and() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "git add . && git commit -m wip",
        &patterns
    ));
}

#[test]
fn test_match_chained_with_semicolon() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "echo hi; git commit -m wip",
        &patterns
    ));
}

#[test]
fn test_match_chained_with_pipe() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "echo msg | git commit --file -",
        &patterns
    ));
}

#[test]
fn test_match_multiline() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "echo hello\ngit commit -m wip",
        &patterns
    ));
}

#[test]
fn test_match_in_dollar_subshell() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "echo $(git commit -m wip)",
        &patterns
    ));
}

#[test]
fn test_match_in_backtick_subshell() {
    let patterns = vec!["git push".into()];
    assert!(command_matches_patterns(
        "echo `git push origin main`",
        &patterns
    ));
}

#[test]
fn test_match_second_backtick_pair() {
    let patterns = vec!["git push".into()];
    assert!(command_matches_patterns(
        "echo `ls` `git push origin main`",
        &patterns
    ));
}

#[test]
fn test_no_match_comment_line() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("# git commit -m wip", &patterns));
}

#[test]
fn test_match_gh_pr_merge_with_admin() {
    let patterns = vec!["gh pr merge".into()];
    assert!(command_matches_patterns(
        "gh pr merge 297 --squash --admin",
        &patterns
    ));
}

#[test]
fn test_no_match_gh_pr_create() {
    let patterns = vec!["gh pr merge".into()];
    assert!(!command_matches_patterns(
        "gh pr create --title foo",
        &patterns
    ));
}

#[test]
fn test_no_match_gh_issue_list() {
    let patterns = vec!["gh pr merge".into()];
    assert!(!command_matches_patterns("gh issue list", &patterns));
}

#[test]
fn test_match_multiple_patterns_first() {
    let patterns = vec!["git commit".into(), "git push".into()];
    assert!(command_matches_patterns("git commit -m wip", &patterns));
}

#[test]
fn test_match_multiple_patterns_second() {
    let patterns = vec!["git commit".into(), "git push".into()];
    assert!(command_matches_patterns("git push origin main", &patterns));
}

#[test]
fn test_no_match_partial_binary() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("gitcommit", &patterns));
}

#[test]
fn test_no_match_partial_subcommand() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns("git committed", &patterns));
}

#[test]
fn test_match_git_commit_amend() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns("git commit --amend", &patterns));
}

#[test]
fn test_match_git_push_force() {
    let patterns = vec!["git push".into()];
    assert!(command_matches_patterns(
        "git push --force origin main",
        &patterns
    ));
}

#[test]
fn test_match_git_merge_no_ff() {
    let patterns = vec!["git merge".into()];
    assert!(command_matches_patterns(
        "git merge --no-ff feature",
        &patterns
    ));
}

#[test]
fn test_match_cargo_publish() {
    let patterns = vec!["cargo publish".into()];
    assert!(command_matches_patterns(
        "cargo publish --dry-run",
        &patterns
    ));
}

#[test]
fn test_no_match_cargo_build() {
    let patterns = vec!["cargo publish".into()];
    assert!(!command_matches_patterns(
        "cargo build --release",
        &patterns
    ));
}

#[test]
fn test_match_npm_publish() {
    let patterns = vec!["npm publish".into()];
    assert!(command_matches_patterns(
        "npm publish --access public",
        &patterns
    ));
}

#[test]
fn test_match_docker_push() {
    let patterns = vec!["docker push".into()];
    assert!(command_matches_patterns(
        "docker push myimage:latest",
        &patterns
    ));
}

#[test]
fn test_match_kubectl_apply() {
    let patterns = vec!["kubectl apply".into()];
    assert!(command_matches_patterns(
        "kubectl apply -f deploy.yaml",
        &patterns
    ));
}

#[test]
fn test_no_match_kubectl_get() {
    let patterns = vec!["kubectl apply".into()];
    assert!(!command_matches_patterns("kubectl get pods", &patterns));
}

#[test]
fn test_match_in_chained_subshell() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "echo $(echo foo; git commit -m x)",
        &patterns
    ));
}

#[test]
fn test_match_case_insensitive_in_subshell() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "echo $(GIT COMMIT -m x)",
        &patterns
    ));
}

#[test]
fn test_extract_bash_command() {
    let cmd = extract_bash_command(r#"{"command": "git commit -m wip"}"#);
    assert_eq!(cmd, "git commit -m wip");
}

#[test]
fn test_extract_bash_command_missing() {
    let cmd = extract_bash_command(r#"{"path": "/tmp"}"#);
    assert_eq!(cmd, "");
}

#[test]
fn test_extract_bash_command_invalid_json() {
    let cmd = extract_bash_command("not json");
    assert_eq!(cmd, "");
}

#[test]
fn test_match_single_word_pattern() {
    let patterns = vec!["rm".into()];
    assert!(command_matches_patterns("rm -rf /", &patterns));
}

#[test]
fn test_no_match_single_word_different_binary() {
    let patterns = vec!["rm".into()];
    assert!(!command_matches_patterns("ls -la", &patterns));
}

#[test]
fn test_match_work_tree_flag() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "git --work-tree /tmp commit -m x",
        &patterns
    ));
}

#[test]
fn test_match_gh_pr_merge_delete_branch() {
    let patterns = vec!["gh pr merge".into()];
    assert!(command_matches_patterns(
        "gh pr merge 42 --squash --delete-branch --admin",
        &patterns
    ));
}

#[test]
fn test_no_match_gh_pr_view() {
    let patterns = vec!["gh pr merge".into()];
    assert!(!command_matches_patterns("gh pr view 42", &patterns));
}

#[test]
fn test_match_multiple_commands_in_chain() {
    let patterns = vec!["git push".into()];
    assert!(command_matches_patterns(
        "git add . && git commit -m wip && git push origin main",
        &patterns
    ));
}

#[test]
fn test_match_unquoted_args_still_matches() {
    // Unquoted "echo git commit" still matches — the guard can't
    // distinguish bare arguments from commands without a full shell
    // parser. But quoted versions (echo "git commit") are now
    // correctly ignored (#405).
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns("echo git commit", &patterns));
}

#[test]
fn test_match_git_push_no_verify() {
    let patterns = vec!["git push".into()];
    assert!(command_matches_patterns(
        "git push --no-verify origin main",
        &patterns
    ));
}

#[test]
fn test_match_git_commit_no_verify() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        "git commit --no-verify -m wip",
        &patterns
    ));
}

// --- #405: quoted strings should not trigger guard ---

#[test]
fn test_no_match_pattern_inside_single_quotes() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns(
        "curl -d '{\"body\": \"git commit the changes\"}' https://api.example.com",
        &patterns
    ));
}

#[test]
fn test_no_match_pattern_inside_double_quotes() {
    let patterns = vec!["git commit".into()];
    assert!(!command_matches_patterns(
        r#"echo "run git commit to save""#,
        &patterns
    ));
}

#[test]
fn test_match_real_command_after_quoted_string() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        r#"echo "hello world" && git commit -m wip"#,
        &patterns
    ));
}

#[test]
fn test_no_match_pattern_in_heredoc_style_echo() {
    let patterns = vec!["git push".into()];
    assert!(!command_matches_patterns(
        "echo 'To deploy, run git push origin main'",
        &patterns
    ));
}

#[test]
fn test_match_unquoted_command_with_quoted_args() {
    let patterns = vec!["git commit".into()];
    assert!(command_matches_patterns(
        r#"git commit -m "fix the bug""#,
        &patterns
    ));
}

#[test]
fn test_no_match_curl_post_with_json_body() {
    let patterns = vec!["git commit".into(), "git push".into()];
    assert!(!command_matches_patterns(
        r#"curl -X POST -H 'Content-Type: application/json' -d '{"message": "git commit and git push"}' https://api.example.com"#,
        &patterns
    ));
}
