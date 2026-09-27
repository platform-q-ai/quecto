// The `edit` tool's refusals: each names the file, says why nothing was
// changed and what to do next (#2193). Loading the file's text is here too,
// since most refusals come from it.

use std::path::Path;

use tokio::io::AsyncReadExt;

use crate::domain::tool::ToolResult;
use crate::infrastructure::tools::truncate::format_size;

use super::edit::{MAX_EDIT_FILE_BYTES, base_normalise};
use super::edit_indent::{describe_indent, indent_mismatch};
use super::edit_match::{LISTED_MATCHES, Matches};
use super::fs_failure::{Access, explain, not_utf8_text, refused};

/// The file's text, or a refusal that names the file, says why it cannot
/// be edited and what to do instead.
pub(super) async fn load_text(full_path: &Path, path: &str) -> Result<String, ToolResult> {
    let metadata = match tokio::fs::metadata(full_path).await {
        Ok(metadata) => metadata,
        Err(error) => return Err(open_refusal(full_path, path, &error).await),
    };
    match metadata.file_type() {
        kind if kind.is_file() => {}
        kind if kind.is_dir() => {
            return Err(refused(format!(
                "{path} is a directory; edit changes a file. List it with ls to find the file."
            )));
        }
        _ => {
            return Err(refused(format!(
                "{path} is not a regular file; edit changes regular text files only."
            )));
        }
    }
    let file = match metadata.len() {
        len if len <= MAX_EDIT_FILE_BYTES => tokio::fs::File::open(full_path).await,
        len => {
            return Err(too_large(
                path,
                &format_size(usize::try_from(len).unwrap_or(usize::MAX)),
            ));
        }
    };
    // Read at most one byte past the limit: the file may have grown since
    // its metadata was read.
    let mut bytes = Vec::new();
    let read = match file {
        Ok(file) => {
            file.take(MAX_EDIT_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = read {
        return Err(open_refusal(full_path, path, &error).await);
    }
    let bytes = match u64::try_from(bytes.len()).unwrap_or(u64::MAX) {
        len if len <= MAX_EDIT_FILE_BYTES => bytes,
        _ => return Err(too_large(path, &format!("over {}", limit_size()))),
    };
    String::from_utf8(bytes)
        .map_err(|error| refused(not_utf8_text(Access::Edit, path, error.as_bytes())))
}

fn limit_size() -> String {
    format_size(usize::try_from(MAX_EDIT_FILE_BYTES).unwrap_or(usize::MAX))
}

fn too_large(path: &str, size: &str) -> ToolResult {
    refused(format!(
        "{path} is too large to edit ({size}; edit takes files up to {}). Change it with \
         bash, e.g. sed -i, or rewrite it with write.",
        limit_size()
    ))
}

/// Why the file could not be opened or read, in words the model can act on.
async fn open_refusal(full_path: &Path, path: &str, error: &std::io::Error) -> ToolResult {
    refused(explain(Access::Edit, full_path, path, error).await)
}

/// The file's own line breaks, as offsets into its normalised text
/// (`base_normalise`): a lone `\r` becomes `\n` there but starts no line in
/// the file, so line numbers match what `read` shows.
#[derive(Debug)]
pub(super) struct FileLines {
    breaks: Vec<usize>,
}

impl FileLines {
    pub(super) fn of(raw: &str) -> Self {
        let body = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
        let mut breaks = Vec::new();
        let mut at = 0usize;
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' if chars.peek() == Some(&'\n') => {
                    chars.next();
                    breaks.push(at);
                    at += 1;
                }
                '\n' => {
                    breaks.push(at);
                    at += 1;
                }
                '\r' => at += 1,
                other => at += other.len_utf8(),
            }
        }
        debug_assert_eq!(at, base_normalise(raw).len());
        Self { breaks }
    }

    /// The 1-based file line holding `offset` of the normalised text.
    pub(super) fn line_of(&self, offset: usize) -> usize {
        self.breaks.partition_point(|at| *at < offset) + 1
    }
}

/// oldText matched more than once: how many times, and where.
pub(super) fn ambiguous(file_lines: &FileLines, path: &str, matches: &Matches) -> ToolResult {
    debug_assert!(matches.count >= 2, "{matches:?}");
    let mut lines: Vec<usize> = matches
        .first
        .iter()
        .map(|range| file_lines.line_of(range.start))
        .collect();
    lines.dedup();
    let lines_text = match lines.as_slice() {
        [one] => format!("line {one}"),
        [init @ .., last] => format!(
            "lines {} and {last}",
            init.iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        [] => "unknown lines".to_string(),
    };
    let place = match (matches.count > LISTED_MATCHES, lines.len()) {
        (true, _) => format!("the first {LISTED_MATCHES} on {lines_text}"),
        (false, 1) => format!("all on {lines_text}"),
        (false, _) => format!("on {lines_text}"),
    };
    let overlapping = if matches.overlapping {
        " (overlapping)"
    } else {
        ""
    };
    refused(format!(
        "oldText matches {} times in {path}{overlapping}, {place} — it must match exactly \
         once to avoid ambiguous edits. Add surrounding lines to oldText to make it unique.",
        matches.count
    ))
}

/// oldText matched nowhere: point at a match with other indentation when
/// there is one, else say where to copy oldText from.
pub(super) fn not_found(
    content: &str,
    file_lines: &FileLines,
    old: &str,
    path: &str,
) -> ToolResult {
    let Some(near) = indent_mismatch(content, old) else {
        return refused(format!(
            "oldText not found in {path}. Read the file and copy oldText from it exactly, \
             whitespace included."
        ));
    };
    let starts: Vec<usize> = std::iter::once(near.start_at)
        .chain(near.also_at.iter().copied())
        .map(|at| file_lines.line_of(at))
        .collect();
    let (place, unique) = match (starts.as_slice(), near.more) {
        ([one], false) => (format!("at line {one}"), String::new()),
        (many, more) => {
            let (init, last) = many.split_at(many.len().saturating_sub(1));
            let list = init
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let last = last.first().copied().unwrap_or_default();
            let place = match (more, list.is_empty()) {
                (true, true) => format!("at line {last} and more"),
                (true, false) => format!("at lines {list}, {last} and more"),
                (false, true) => format!("at line {last}"),
                (false, false) => format!("at lines {list} and {last}"),
            };
            (
                place,
                " and add surrounding lines to make it unique".to_string(),
            )
        }
    };
    refused(format!(
        "oldText not found in {path}, but it matches {place} if indentation is ignored; \
         line {} is indented with {} in the file, {} in oldText. Copy the indentation \
         from the file{unique}.",
        file_lines.line_of(near.differs_at),
        describe_indent(&near.file_indent),
        describe_indent(&near.old_indent)
    ))
}

#[cfg(test)]
#[path = "edit_refusal_tests.rs"]
mod tests;
