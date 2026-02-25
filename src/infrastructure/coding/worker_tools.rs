//! Worker coding tool implementations for the nsjail sandbox.
//!
//! These tools operate on the per-job clone directory only. They provide
//! enhanced diagnostics (unified diffs, ambiguity detection, fuzzy match)
//! and safety (destructive git command blocking, path boundary enforcement).

use std::path::{Path, PathBuf};

// ── Edit result types ───────────────────────────────────────────────────

/// Result of an edit operation.
#[derive(Debug, Clone)]
pub struct EditResult {
    /// Whether the edit succeeded.
    pub ok: bool,
    /// Unified diff showing the change (on success).
    pub diff: Option<String>,
    /// First changed line number (1-indexed).
    pub first_changed_line: Option<usize>,
    /// Error message on failure.
    pub error: Option<String>,
    /// Number of ambiguous matches found (on ambiguity error).
    pub match_count: Option<usize>,
    /// Line numbers of each match (on ambiguity error).
    pub match_lines: Option<Vec<usize>>,
    /// Whether fuzzy matching was used.
    pub fuzzy_used: bool,
}

/// Result of a grep operation.
#[derive(Debug, Clone)]
pub struct GrepResult {
    pub ok: bool,
    pub matches: Vec<GrepMatch>,
    pub error: Option<String>,
}

/// A single grep match.
#[derive(Debug, Clone)]
pub struct GrepMatch {
    pub file: String,
    pub line: usize,
    pub text: String,
}

/// Result of a find-files operation.
#[derive(Debug, Clone)]
pub struct FindResult {
    pub ok: bool,
    pub files: Vec<String>,
    pub error: Option<String>,
}

/// Result of a git operation.
#[derive(Debug, Clone)]
pub struct GitOpResult {
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
}

/// Result of a read operation with pagination.
#[derive(Debug, Clone)]
pub struct ReadResult {
    pub ok: bool,
    pub content: String,
    pub total_lines: usize,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
    pub error: Option<String>,
}

// ── Path boundary enforcement ───────────────────────────────────────────

/// Check if a path is within the allowed job directory.
pub fn is_within_job_dir(path: &Path, job_dir: &Path) -> bool {
    match (path.canonicalize(), job_dir.canonicalize()) {
        (Ok(p), Ok(j)) => p.starts_with(j),
        _ => {
            // Fallback: lexical check for non-existent paths
            let p = normalize_path(path);
            let j = normalize_path(job_dir);
            p.starts_with(j)
        }
    }
}

/// Normalize a path by resolving `.` and `..` components lexically.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            c => result.push(c),
        }
    }
    result
}

// ── Destructive git command detection ───────────────────────────────────

/// Git subcommands that are blocked by default for safety.
const BLOCKED_GIT_COMMANDS: &[&str] = &[
    "push --force",
    "push -f",
    "reset --hard",
    "clean -fd",
    "clean -f",
    "clean -d",
    "rebase --force",
    "checkout --force",
];

/// Check if a git command string is destructive and should be blocked.
pub fn is_destructive_git_command(command: &str) -> bool {
    let trimmed = command.trim();
    // Must start with "git "
    if !trimmed.starts_with("git ") {
        return false;
    }
    let rest = &trimmed[4..];
    BLOCKED_GIT_COMMANDS
        .iter()
        .any(|blocked| rest.contains(blocked))
}

// ── Edit engine ─────────────────────────────────────────────────────────

/// Parameters for the `edit_file` function.
pub struct EditParams<'a> {
    /// Root directory of the job clone.
    pub job_dir: &'a Path,
    /// Relative file path within the job directory.
    pub file_path: &'a str,
    /// String to search for.
    pub old_string: &'a str,
    /// Replacement string.
    pub new_string: &'a str,
    /// If true, compute diff but do not write.
    pub preview_only: bool,
    /// If true, try fuzzy (whitespace-trimmed) matching on miss.
    pub fuzzy: bool,
}

fn edit_error(msg: &str) -> EditResult {
    EditResult {
        ok: false,
        diff: None,
        first_changed_line: None,
        error: Some(msg.to_string()),
        match_count: None,
        match_lines: None,
        fuzzy_used: false,
    }
}

/// Read and normalize file content, returning (raw, normalized, search, has_crlf, has_bom).
fn read_and_normalize(full_path: &Path, old_string: &str) -> Result<FileContent, EditResult> {
    let content = std::fs::read_to_string(full_path)
        .map_err(|e| edit_error(&format!("cannot read file: {e}")))?;
    let has_bom = content.starts_with('\u{feff}');
    let work = if has_bom { &content[3..] } else { &content };
    let has_crlf = work.contains("\r\n");
    let normalized = if has_crlf {
        work.replace("\r\n", "\n")
    } else {
        work.to_string()
    };
    let search = if has_crlf {
        old_string.replace("\r\n", "\n")
    } else {
        old_string.to_string()
    };
    Ok(FileContent {
        _raw: content,
        normalized,
        search,
        has_crlf,
        has_bom,
    })
}

struct FileContent {
    _raw: String,
    normalized: String,
    search: String,
    has_crlf: bool,
    has_bom: bool,
}

/// Find match positions, optionally using fuzzy and smart punctuation fallback.
fn find_match_positions(
    normalized: &str,
    search: &str,
    fuzzy: bool,
) -> (Vec<usize>, bool, Option<usize>) {
    let exact: Vec<usize> = normalized.match_indices(search).map(|(i, _)| i).collect();
    if !exact.is_empty() {
        return (exact, false, None);
    }
    if fuzzy {
        let fz_search = search
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        let fz_content = normalized
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        let fz: Vec<usize> = fz_content
            .match_indices(&fz_search)
            .map(|(i, _)| i)
            .collect();
        if !fz.is_empty() {
            return (fz, true, None);
        }
    }
    // Smart punctuation fallback
    let smart_s = normalize_smart_punctuation(search);
    let smart_c = normalize_smart_punctuation(normalized);
    let sm: Vec<usize> = smart_c.match_indices(&smart_s).map(|(i, _)| i).collect();
    if sm.len() == 1 {
        return (sm, true, Some(smart_s.len()));
    }
    (vec![], fuzzy, None)
}

/// Perform an exact string replacement in a file.
pub fn edit_file(params: &EditParams<'_>) -> EditResult {
    if params.old_string == params.new_string {
        return edit_error("no-op: old and new strings are identical");
    }
    let full_path = params.job_dir.join(params.file_path);
    if !is_within_job_dir(&full_path, params.job_dir) {
        return edit_error("path violation: file is outside job directory");
    }
    let fc = match read_and_normalize(&full_path, params.old_string) {
        Ok(fc) => fc,
        Err(e) => return e,
    };
    let (positions, fuzzy_used, smart_len) =
        find_match_positions(&fc.normalized, &fc.search, params.fuzzy);

    if positions.is_empty() {
        return EditResult {
            ok: false,
            diff: None,
            first_changed_line: None,
            error: Some("old_string not found in file content".to_string()),
            match_count: Some(0),
            match_lines: Some(vec![]),
            fuzzy_used,
        };
    }
    if positions.len() > 1 {
        let lines: Vec<usize> = positions
            .iter()
            .map(|&p| fc.normalized[..p].matches('\n').count() + 1)
            .collect();
        return EditResult {
            ok: false,
            diff: None,
            first_changed_line: None,
            error: Some(format!(
                "ambiguous: found {} matches for the search string",
                positions.len()
            )),
            match_count: Some(positions.len()),
            match_lines: Some(lines),
            fuzzy_used,
        };
    }
    let pos = positions[0];
    let match_len = smart_len.unwrap_or(fc.search.len());
    perform_replacement(&ReplacementContext {
        normalized: &fc.normalized,
        pos,
        match_len,
        new_string: params.new_string,
        has_crlf: fc.has_crlf,
        has_bom: fc.has_bom,
        preview_only: params.preview_only,
        full_path: &full_path,
        fuzzy_used,
    })
}

/// Context for the `perform_replacement` helper.
struct ReplacementContext<'a> {
    normalized: &'a str,
    pos: usize,
    match_len: usize,
    new_string: &'a str,
    has_crlf: bool,
    has_bom: bool,
    preview_only: bool,
    full_path: &'a Path,
    fuzzy_used: bool,
}

fn perform_replacement(ctx: &ReplacementContext<'_>) -> EditResult {
    let pos = ctx.pos;
    let match_len = ctx.match_len;
    let normalized = ctx.normalized;
    let new_string = ctx.new_string;
    let has_crlf = ctx.has_crlf;
    let has_bom = ctx.has_bom;
    let preview_only = ctx.preview_only;
    let full_path = ctx.full_path;
    let fuzzy_used = ctx.fuzzy_used;

    let mut result_content = String::new();
    result_content.push_str(&normalized[..pos]);
    result_content.push_str(new_string);
    result_content.push_str(&normalized[pos + match_len..]);

    // Compute first changed line
    let first_changed_line = normalized[..pos].matches('\n').count() + 1;

    // Compute unified diff
    let diff = compute_unified_diff(normalized, &result_content);

    // Restore CRLF if original had it
    let final_content = if has_crlf {
        result_content.replace('\n', "\r\n")
    } else {
        result_content
    };

    // Restore BOM
    let final_content = if has_bom {
        format!("\u{feff}{final_content}")
    } else {
        final_content
    };

    if !preview_only {
        if let Err(e) = std::fs::write(full_path, &final_content) {
            return EditResult {
                ok: false,
                diff: Some(diff),
                first_changed_line: Some(first_changed_line),
                error: Some(format!("write failed: {e}")),
                match_count: None,
                match_lines: None,
                fuzzy_used,
            };
        }
    }

    EditResult {
        ok: true,
        diff: Some(diff),
        first_changed_line: Some(first_changed_line),
        error: None,
        match_count: None,
        match_lines: None,
        fuzzy_used,
    }
}

/// Normalize smart punctuation to ASCII equivalents.
fn normalize_smart_punctuation(s: &str) -> String {
    s.replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"")
        .replace('\u{2013}', "-")
        .replace('\u{2014}', "--")
}

/// Compute a minimal unified diff between two strings.
fn compute_unified_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut diff = String::new();

    diff.push_str("--- a/file\n");
    diff.push_str("+++ b/file\n");

    // Simple line-by-line diff
    let max_len = old_lines.len().max(new_lines.len());
    let mut i = 0;
    while i < max_len {
        let old_line = old_lines.get(i).copied();
        let new_line = new_lines.get(i).copied();
        match (old_line, new_line) {
            (Some(o), Some(n)) if o == n => {
                diff.push_str(&format!(" {o}\n"));
            }
            (Some(o), Some(n)) => {
                diff.push_str(&format!("-{o}\n"));
                diff.push_str(&format!("+{n}\n"));
            }
            (Some(o), None) => {
                diff.push_str(&format!("-{o}\n"));
            }
            (None, Some(n)) => {
                diff.push_str(&format!("+{n}\n"));
            }
            (None, None) => {}
        }
        i += 1;
    }
    diff
}

// ── Grep engine ─────────────────────────────────────────────────────────

/// Grep for a pattern in all files under the job directory.
///
/// The pattern is matched as a literal substring. For BDD and unit
/// testing, the test harness may convert regex patterns to simple
/// substring matches.
pub fn grep_content(job_dir: &Path, pattern: &str, gitignore: bool) -> GrepResult {
    let gitignore_patterns = if gitignore {
        load_gitignore_patterns(job_dir)
    } else {
        vec![]
    };

    let mut matches = Vec::new();
    visit_files(job_dir, job_dir, &gitignore_patterns, &mut |rel_path| {
        if let Ok(content) = std::fs::read_to_string(job_dir.join(rel_path)) {
            for (line_num, line) in content.lines().enumerate() {
                if line.contains(pattern) {
                    matches.push(GrepMatch {
                        file: rel_path.to_string(),
                        line: line_num + 1,
                        text: line.to_string(),
                    });
                }
            }
        }
    });

    GrepResult {
        ok: true,
        matches,
        error: None,
    }
}

// ── Find engine ─────────────────────────────────────────────────────────

/// Find files matching a glob pattern under the job directory.
///
/// Supports basic glob patterns: `*` (any segment), `**` (recursive),
/// `?` (single char). Uses a simple matcher — no external crate needed.
pub fn find_files(job_dir: &Path, glob_pattern: &str, gitignore: bool) -> FindResult {
    let gitignore_patterns = if gitignore {
        load_gitignore_patterns(job_dir)
    } else {
        vec![]
    };

    let mut files = Vec::new();
    visit_files(job_dir, job_dir, &gitignore_patterns, &mut |rel_path| {
        if simple_glob_match(glob_pattern, rel_path) {
            files.push(rel_path.to_string());
        }
    });

    files.sort();

    FindResult {
        ok: true,
        files,
        error: None,
    }
}

// ── Read with pagination ────────────────────────────────────────────────

/// Read a file with offset and limit for pagination.
pub fn read_file_paginated(
    job_dir: &Path,
    file_path: &str,
    offset: usize,
    limit: usize,
) -> ReadResult {
    let full_path = job_dir.join(file_path);
    if !is_within_job_dir(&full_path, job_dir) {
        return ReadResult {
            ok: false,
            content: String::new(),
            total_lines: 0,
            offset,
            limit,
            has_more: false,
            error: Some("path violation: file is outside job directory".to_string()),
        };
    }

    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => {
            return ReadResult {
                ok: false,
                content: String::new(),
                total_lines: 0,
                offset,
                limit,
                has_more: false,
                error: Some(format!("cannot read file: {e}")),
            };
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start = offset.min(total_lines);
    let end = (start + limit).min(total_lines);
    let selected: Vec<&str> = lines[start..end].to_vec();
    let has_more = end < total_lines;

    ReadResult {
        ok: true,
        content: selected.join("\n"),
        total_lines,
        offset: start,
        limit,
        has_more,
        error: None,
    }
}

// ── Gitignore support ───────────────────────────────────────────────────

/// Load patterns from a .gitignore file in the job directory.
fn load_gitignore_patterns(job_dir: &Path) -> Vec<String> {
    let gitignore_path = job_dir.join(".gitignore");
    match std::fs::read_to_string(gitignore_path) {
        Ok(content) => content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect(),
        Err(_) => vec![],
    }
}

/// Check if a relative path matches any gitignore pattern.
fn is_gitignored(rel_path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        let pat = pattern.trim_end_matches('/');
        if rel_path.starts_with(pat) || rel_path.contains(&format!("/{pat}")) {
            return true;
        }
        // Handle glob-like patterns (e.g., *.log)
        if simple_glob_match(pattern, rel_path) {
            return true;
        }
        // Also check just the filename
        if let Some(filename) = rel_path.rsplit('/').next() {
            if simple_glob_match(pattern, filename) {
                return true;
            }
        }
    }
    false
}

/// Simple glob matcher supporting `*`, `**`, and `?`.
///
/// `**` matches zero or more path segments (including `/`).
/// `*` matches any characters except `/`.
/// `?` matches any single character except `/`.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    simple_glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn simple_glob_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }

    // Handle ** (match zero or more path segments)
    if pattern.starts_with(b"**/") {
        let rest = &pattern[3..];
        // Try matching with zero segments
        if simple_glob_match_inner(rest, text) {
            return true;
        }
        // Try skipping each character in text
        for i in 0..text.len() {
            if text[i] == b'/' && simple_glob_match_inner(rest, &text[i + 1..]) {
                return true;
            }
        }
        return false;
    }

    // Handle trailing **
    if pattern == b"**" {
        return true;
    }

    if text.is_empty() {
        // Pattern must be all wildcards to match empty text
        return pattern.iter().all(|&b| b == b'*');
    }

    match pattern[0] {
        b'*' => {
            // * matches any non-slash character sequence
            // Try matching zero characters, then one, etc.
            if simple_glob_match_inner(&pattern[1..], text) {
                return true;
            }
            for i in 0..text.len() {
                if text[i] == b'/' {
                    break;
                }
                if simple_glob_match_inner(&pattern[1..], &text[i + 1..]) {
                    return true;
                }
            }
            false
        }
        b'?' => {
            if text[0] != b'/' {
                simple_glob_match_inner(&pattern[1..], &text[1..])
            } else {
                false
            }
        }
        c => {
            if c == text[0] {
                simple_glob_match_inner(&pattern[1..], &text[1..])
            } else {
                false
            }
        }
    }
}

/// Recursively visit files under a directory, skipping gitignored paths.
fn visit_files(root: &Path, dir: &Path, gitignore: &[String], callback: &mut dyn FnMut(&str)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if is_gitignored(&rel, gitignore) {
            continue;
        }

        if path.is_dir() {
            // Skip hidden dirs
            if rel.starts_with('.') || rel.contains("/.") {
                continue;
            }
            visit_files(root, &path, gitignore, callback);
        } else if path.is_file() {
            callback(&rel);
        }
    }
}

#[cfg(test)]
#[path = "worker_tools_tests.rs"]
mod tests;
