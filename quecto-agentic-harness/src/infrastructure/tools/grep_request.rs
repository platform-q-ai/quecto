//! The grep tool's arguments, validated into one request (#2136): the rg
//! options agents otherwise reach for in bash — several patterns, several
//! globs, file types, whole words, multiline, a per-file cap, and files or
//! counts instead of lines. Every argument is accepted only in a shape the
//! tool can honour; anything else is refused with the valid choices.

use serde_json::Value;

/// Maximum number of matches (or files) to return by default.
pub(super) const DEFAULT_MATCH_LIMIT: usize = 100;
/// Maximum context lines per side; prevents unbounded file reads.
pub(super) const MAX_CONTEXT_LINES: usize = 50;

/// What the search returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputMode {
    /// Matching lines, with optional context (the default).
    Content,
    /// Each matching file once (rg `-l`).
    Files,
    /// Each matching file with its match count, busiest first (rg `-c`).
    Count,
}

/// One validated search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GrepRequest {
    pub patterns: Vec<String>,
    pub path: String,
    pub globs: Vec<String>,
    pub types: Vec<String>,
    pub ignore_case: bool,
    pub literal: bool,
    pub word: bool,
    pub multiline: bool,
    pub max_per_file: Option<u64>,
    pub output: OutputMode,
    pub context_lines: usize,
    pub limit: usize,
}

const EXAMPLE: &str = r#"Example: {"pattern": "search_term"}"#;

/// Validate the model's arguments into a request, or say what is wrong.
pub(super) fn parse_request(args: &Value) -> Result<GrepRequest, String> {
    let patterns = patterns(args)?;
    let request = GrepRequest {
        patterns,
        path: optional_string(args, "path")?.unwrap_or_else(|| ".".to_string()),
        globs: string_or_strings(args, "glob")?,
        types: string_or_strings(args, "type")?,
        ignore_case: flag(args, "ignoreCase")?,
        literal: flag(args, "literal")?,
        word: flag(args, "wordRegexp")?,
        multiline: flag(args, "multiline")?,
        max_per_file: whole_number(args, "maxPerFile", 1)?,
        output: output_mode(args)?,
        context_lines: whole_number(args, "context", 0)?
            .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX))
            .min(MAX_CONTEXT_LINES),
        limit: whole_number(args, "limit", 1)?.map_or(DEFAULT_MATCH_LIMIT, |n| {
            usize::try_from(n).unwrap_or(usize::MAX)
        }),
    };
    debug_assert!(
        !request.patterns.is_empty(),
        "a request always searches for something"
    );
    Ok(request)
}

/// `pattern` (a string) and/or `patterns` (strings): at least one, none empty.
fn patterns(args: &Value) -> Result<Vec<String>, String> {
    let mut all = Vec::new();
    all.extend(optional_string(args, "pattern")?);
    all.extend(match &args["patterns"] {
        Value::Null => Vec::new(),
        Value::Array(items) => strings(items, "patterns")?,
        _ => return Err(format!("patterns must be an array of strings. {EXAMPLE}")),
    });
    if !all.is_empty() && all.iter().all(|p| !p.is_empty()) {
        Ok(all)
    } else {
        Err(format!(
            "missing 'pattern' argument (a non-empty string, or 'patterns': [..]). {EXAMPLE}"
        ))
    }
}

fn optional_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match &args[key] {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(format!("{key} must be a string. {EXAMPLE}")),
    }
}

/// A string or an array of strings (rg repeats the flag for each).
fn string_or_strings(args: &Value, key: &str) -> Result<Vec<String>, String> {
    match &args[key] {
        Value::Null => Ok(Vec::new()),
        Value::String(s) if !s.is_empty() => Ok(vec![s.clone()]),
        Value::Array(items) => strings(items, key),
        _ => Err(format!(
            "{key} must be a non-empty string or an array of them"
        )),
    }
}

fn strings(items: &[Value], key: &str) -> Result<Vec<String>, String> {
    items
        .iter()
        .map(|item| match item {
            Value::String(s) if !s.is_empty() => Ok(s.clone()),
            _ => Err(format!("{key} must hold only non-empty strings")),
        })
        .collect()
}

fn flag(args: &Value, key: &str) -> Result<bool, String> {
    match &args[key] {
        Value::Null => Ok(false),
        Value::Bool(b) => Ok(*b),
        _ => Err(format!("{key} must be true or false")),
    }
}

/// A number of at least `min`, rounded (models send numbers as floats).
fn whole_number(args: &Value, key: &str, min: u64) -> Result<Option<u64>, String> {
    let refused = || format!("{key} must be a whole number of at least {min}");
    match &args[key] {
        Value::Null => Ok(None),
        Value::Number(n) => match n.as_f64().map(f64::round) {
            Some(v) if v.is_finite() && v >= min as f64 => Ok(Some(if v >= u64::MAX as f64 {
                u64::MAX
            } else {
                v as u64
            })),
            _ => Err(refused()),
        },
        _ => Err(refused()),
    }
}

fn output_mode(args: &Value) -> Result<OutputMode, String> {
    match &args["output"] {
        Value::Null => Ok(OutputMode::Content),
        Value::String(s) => match s.as_str() {
            "content" => Ok(OutputMode::Content),
            "files" => Ok(OutputMode::Files),
            "count" => Ok(OutputMode::Count),
            other => Err(format!(
                "output must be one of content, files, count (got '{other}')"
            )),
        },
        _ => Err("output must be one of content, files, count".to_string()),
    }
}

#[cfg(test)]
#[path = "grep_request_tests.rs"]
mod tests;
