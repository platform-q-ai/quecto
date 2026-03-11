//! Command pattern matching for workflow guards.
//!
//! Matches bash commands against configurable patterns like `"git commit"`,
//! `"gh pr merge"`, `"cargo publish"`. Handles chained commands, flags,
//! subshells, and backtick expressions.

/// Extract the command string from bash tool JSON arguments.
pub(crate) fn extract_bash_command(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
        .unwrap_or_default()
}

/// Check if a bash command matches any of the given command patterns.
///
/// Each pattern is `"binary subcommand"` (e.g. `"git commit"`, `"gh pr merge"`).
/// Handles chained commands (`&&`, `||`, `;`, newlines), flags between
/// binary and subcommand, and subshells (`$(...)`, backticks).
/// Convenience wrapper that parses patterns and matches in one call.
/// Used by tests only; production code uses `parse_patterns` + `command_matches_parsed`
/// to avoid per-call allocation.
#[cfg(test)]
pub fn command_matches_patterns(command: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }

    let parsed = parse_patterns(patterns);
    command_matches_parsed(command, &parsed)
}

/// Parse command pattern strings into (binary, subcmd_tokens) pairs.
pub(crate) fn parse_patterns(patterns: &[String]) -> Vec<(String, Vec<String>)> {
    patterns
        .iter()
        .map(|p| {
            let parts: Vec<&str> = p.split_whitespace().collect();
            let binary = parts.first().unwrap_or(&"").to_lowercase();
            let subcmds: Vec<String> = parts.iter().skip(1).map(|s| s.to_lowercase()).collect();
            (binary, subcmds)
        })
        .collect()
}

/// Strip content inside single and double quotes, replacing with spaces.
///
/// Handles nested quotes (single inside double and vice versa) by only
/// tracking the outermost quote character. Escaped quotes (`\"`, `\'`)
/// inside double-quoted regions are skipped.
fn strip_quoted_regions(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_quote: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        match in_quote {
            None => {
                if b == b'\'' || b == b'"' {
                    in_quote = Some(b);
                    out.push(' '); // placeholder so tokens don't merge
                } else {
                    out.push(b as char);
                }
            }
            Some(q) => {
                if b == b'\\' && q == b'"' && i + 1 < bytes.len() {
                    // Skip escaped char inside double quotes
                    i += 2;
                    continue;
                }
                if b == q {
                    in_quote = None;
                }
                // Consume quoted content (don't emit it)
            }
        }
        i += 1;
    }
    out
}

/// Inner matching logic against pre-parsed patterns.
pub(crate) fn command_matches_parsed(command: &str, patterns: &[(String, Vec<String>)]) -> bool {
    // Strip quoted regions first so data inside strings doesn't trigger
    // false positives (#405).
    let unquoted = strip_quoted_regions(command);
    // Lowercase once for case-insensitive matching at all levels
    let lower = unquoted.to_lowercase();

    for segment in lower.split(&['&', '|', ';', '\n'][..]) {
        let segment = segment.trim();
        if segment.is_empty() || segment.starts_with('#') {
            continue;
        }

        if segment_matches_any(segment, patterns) {
            return true;
        }
    }

    contains_in_subshell_lowered(&lower, patterns)
}

/// Check if a single command segment matches any pattern.
fn segment_matches_any(segment: &str, patterns: &[(String, Vec<String>)]) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();

    for (binary, subcmds) in patterns {
        if matches_binary_subcommand(&tokens, binary, subcmds) {
            return true;
        }
    }
    false
}

/// Check if tokens contain `binary [flags...] subcommand_tokens...`.
fn matches_binary_subcommand(tokens: &[&str], binary: &str, subcmds: &[String]) -> bool {
    let mut found_binary = false;
    let mut skip_next = false;
    let mut subcmd_idx = 0;

    for token in tokens {
        if skip_next {
            skip_next = false;
            continue;
        }
        if !found_binary {
            if *token == binary {
                if subcmds.is_empty() {
                    return true;
                }
                found_binary = true;
                subcmd_idx = 0;
            }
            continue;
        }
        // After finding binary, skip flags
        if token.starts_with('-') {
            if *token == "-c" || *token == "-C" || *token == "--git-dir" || *token == "--work-tree"
            {
                skip_next = true;
            }
            continue;
        }
        // Match subcommand tokens in order
        if subcmd_idx < subcmds.len() && *token == subcmds[subcmd_idx] {
            subcmd_idx += 1;
            if subcmd_idx == subcmds.len() {
                return true;
            }
        } else {
            found_binary = false;
        }
    }
    false
}

/// Detect patterns inside subshells: `$(...)` and backtick expressions.
/// Input should already be lowercased.
fn contains_in_subshell_lowered(lower: &str, patterns: &[(String, Vec<String>)]) -> bool {
    let mut pos = 0;
    while let Some(start) = lower[pos..].find("$(") {
        let abs_start = pos + start + 2;
        if let Some(end) = lower[abs_start..].find(')') {
            let inside = &lower[abs_start..abs_start + end];
            if command_matches_parsed(inside, patterns) {
                return true;
            }
            pos = abs_start + end + 1;
        } else {
            break;
        }
    }

    let mut pos = 0;
    while let Some(start) = lower[pos..].find('`') {
        let abs_start = pos + start + 1;
        if let Some(end) = lower[abs_start..].find('`') {
            let inside = &lower[abs_start..abs_start + end];
            if command_matches_parsed(inside, patterns) {
                return true;
            }
            pos = abs_start + end + 1;
        } else {
            break;
        }
    }

    false
}

#[cfg(test)]
mod tests {
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
}
