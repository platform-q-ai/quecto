//! Strip chainlink-style `[noun]` step tags from Gherkin `.feature` files
//! before cucumber-rs sees them.
//!
//! Why this exists: `chainlink tag --auto` inserts explicit domain-noun tags
//! into step text (e.g. `Then the [ToolResult] should contain ...`). These
//! tags are useful for static analysis — they give chainlink an authoritative
//! feature → noun link. But cucumber-rs matches step text literally against
//! the regex / expr strings on `#[given|when|then]` attributes, which are
//! written against the bare prose form (`the tool result should contain ...`).
//! The two conventions collide at the `.feature` layer.
//!
//! This preprocessor decouples them: the `.feature` files on disk keep their
//! `[Foo]` brackets (so chainlink's chain-traversal stays accurate), and
//! cucumber sees a transformed copy with the brackets stripped (so step
//! matching works).
//!
//! Only step lines are touched — `Given`, `When`, `Then`, `And`, `But`. Tag
//! lines (`@foo`), `Feature:` / `Scenario:` headers, docstring contents
//! (`"""..."""`), data-table rows (`|...|`), and comments are left verbatim.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Mirror the tree under `source` into `target`, rewriting every `.feature`
/// file so bracketed noun tags in step lines become bare prose (CamelCase
/// chainlink noun names converted back to the spaced-lowercase form that
/// cucumber-rs matchers are written against). Non-step lines and non-.feature
/// files are copied verbatim.
pub fn prepare_stripped_features(source: &Path, target: &Path) -> std::io::Result<()> {
    let bracket_re = Regex::new(r"\[([\w\-]+)\]").unwrap();
    mirror_dir(source, target, &bracket_re)
}

/// Walk the line rewriting each `[tag]` in place. A CamelCase tag preceded
/// by `the ` expands to its spaced-lowercase prose form; any other tag just
/// has its brackets stripped (preserving whatever was inside).
fn rewrite_step_tags(line: &str, re: &Regex) -> String {
    let mut out = String::with_capacity(line.len());
    let mut cursor = 0;
    for mat in re.find_iter(line) {
        out.push_str(&line[cursor..mat.start()]);
        let tag = &line[mat.start() + 1 .. mat.end() - 1];
        let preceded_by_the = out.trim_end().ends_with(" the") || out.trim_end() == "the";
        let is_camel_case = tag.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
            && tag.chars().any(|c| c.is_ascii_lowercase());
        if preceded_by_the && is_camel_case {
            out.push_str(&noun_tag_to_prose(tag));
        } else {
            out.push_str(tag);
        }
        cursor = mat.end();
    }
    out.push_str(&line[cursor..]);
    out
}

/// Convert a chainlink noun tag body to the prose form a cucumber step
/// matcher would be written against:
///
///   `ToolResult`   → `tool result`
///   `LlmProvider`  → `llm provider`   (acronym prefix splits at boundary)
///   `session`      → `session`        (single-word, unchanged)
///   `non-behavioral` → `non-behavioral` (hyphenated, unchanged)
///
/// Kept aligned with `chainlink::scanner::feature::noun_phrase_variants` so a
/// tag round-trips: inferring `ToolResult` from prose "tool result" and then
/// rewriting `[ToolResult]` back to "tool result" both use the same model.
fn noun_tag_to_prose(tag: &str) -> String {
    // If the tag already contains hyphens or underscores, leave it alone —
    // it's a reserved marker (`non-behavioral`) or already in the prose form.
    if tag.contains('-') || tag.contains('_') {
        return tag.to_string();
    }

    // Split on uppercase boundaries. `ToolResult` → ["Tool", "Result"].
    // Acronym prefixes like `LlmProvider` split at the Llm/Provider boundary.
    let chars: Vec<char> = tag.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_lower = i > 0 && chars[i - 1].is_ascii_lowercase();
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_ascii_lowercase();
            let prev_upper = i > 0 && chars[i - 1].is_ascii_uppercase();
            if !current.is_empty() && (prev_lower || (prev_upper && next_lower)) {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c.to_ascii_lowercase());
    }
    if !current.is_empty() { words.push(current); }

    if words.len() <= 1 { return tag.to_ascii_lowercase(); }
    words.join(" ")
}

fn mirror_dir(src: &Path, dst: &Path, re: &Regex) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let ty = entry.file_type()?;
        if ty.is_dir() {
            mirror_dir(&src_path, &dst_path, re)?;
        } else if ty.is_file() {
            let is_feature = src_path.extension().map(|e| e == "feature").unwrap_or(false);
            if is_feature {
                let original = fs::read_to_string(&src_path)?;
                fs::write(&dst_path, strip_step_tags(&original, re))?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }
    }
    Ok(())
}

const STEP_KEYWORDS: &[&str] = &["Given ", "When ", "Then ", "And ", "But "];

/// Strip `[xxx]` from step lines only. Leaves Gherkin structure untouched.
fn strip_step_tags(content: &str, re: &Regex) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_docstring = false;

    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();

        // Docstrings (`"""..."""`) are literal content — don't touch them.
        if trimmed.starts_with("\"\"\"") {
            in_docstring = !in_docstring;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_docstring {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let is_step = STEP_KEYWORDS.iter().any(|kw| trimmed.starts_with(kw));
        if !is_step {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Convert each `[Foo]` to its matcher-compatible form, using the
        // preceding text for context:
        //   "the [ToolResult] should contain"   → "the tool result should contain"  (prose)
        //   "a [ToolCall] event is emitted"     → "a ToolCall event is emitted"     (Rust-name)
        //   "an [AuditEvent]::Variant with …"   → "an AuditEvent::Variant with …"   (Rust-path)
        //   "the [session] is saved"            → "the session is saved"            (unchanged)
        // Heuristic: a CamelCase tag preceded by `the ` is prose — expand to
        // spaced lowercase. Any other context preserves the inner text.
        let rewritten = rewrite_step_tags(line, re);
        out.push_str(&rewritten);
        out.push('\n');

        // Keep the line index silenceable for debugging.
        let _ = i;
    }
    if !content.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Convenience: create a tempdir and populate it with stripped features.
/// The caller must keep the returned `TempDir` alive for the cucumber run.
pub fn stripped_features_tempdir(source: &Path) -> std::io::Result<(tempfile::TempDir, PathBuf)> {
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().to_path_buf();
    prepare_stripped_features(source, &path)?;
    Ok((tmp, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn re() -> Regex { Regex::new(r"\[([\w\-]+)\]").unwrap() }

    #[test]
    fn camelcase_tag_expands_to_spaced_prose() {
        let in_ = "  Then the [ToolResult] should contain \"ok\"\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, "  Then the tool result should contain \"ok\"\n");
    }

    #[test]
    fn single_lowercase_tag_strips_to_itself() {
        let in_ = "  Given the [session] is saved\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, "  Given the session is saved\n");
    }

    #[test]
    fn acronym_prefix_splits_at_boundary() {
        let in_ = "  When the [LlmProvider] responds\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, "  When the llm provider responds\n");
    }

    #[test]
    fn feature_line_brackets_preserved() {
        // Not a step line: leave it alone.
        let in_ = "  Feature: Some [feature]\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, in_);
    }

    #[test]
    fn docstring_contents_preserved() {
        let in_ = "  Given a doc\n    \"\"\"\n    the [ToolResult] is inside\n    \"\"\"\n";
        let out = strip_step_tags(in_, &re());
        assert!(out.contains("[ToolResult] is inside"),
            "docstring content must not be stripped, got:\n{out}");
    }

    #[test]
    fn data_table_rows_preserved() {
        // Pipe-prefixed lines aren't step lines; leave them alone even if
        // they contain bracketed text.
        let in_ = "    | field | [value] |\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, in_);
    }

    #[test]
    fn multiple_tags_per_line_all_expanded() {
        let in_ = "  When the [User] places an [Order]\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, "  When the user places an order\n");
    }

    #[test]
    fn hyphenated_tags_supported() {
        let in_ = "  Given a [non-behavioral] marker\n";
        let out = strip_step_tags(in_, &re());
        assert_eq!(out, "  Given a non-behavioral marker\n");
    }
}
