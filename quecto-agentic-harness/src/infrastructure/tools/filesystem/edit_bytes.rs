// The `edit` tool's view of the file's own bytes (#2242). Matching runs on
// the normalised text (`base_normalise`), but the edit is spliced into the
// file's text as it is, so every byte outside the matched span — its BOM,
// its line endings, a lone `\r` — is written back unchanged.
//
// Pure text logic, no I/O.

/// The line ending newText's line breaks are written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineEnding {
    Lf,
    Crlf,
}

/// The ending of the last line `span` of `raw` replaces (#2242 review):
/// of its last line break when it ends with one, else of the line it ends
/// on, else of its own last line break, else of the line before it, else
/// LF. New lines beyond the replaced ones take it ([`with_span_endings`]).
pub(super) fn span_line_ending(raw: &str, span: std::ops::Range<usize>) -> LineEnding {
    let inside = raw.get(span.clone()).unwrap_or_default();
    debug_assert_eq!(
        inside.len(),
        span.len(),
        "{span:?} is not a span of the text"
    );
    let own = line_endings(inside);
    let next = raw.get(span.end..).and_then(|rest| rest.find('\n'));
    let before = raw.get(..span.start).and_then(|head| head.rfind('\n'));
    match (inside.ends_with('\n'), own.last(), next, before) {
        (true, Some(last), _, _) => *last,
        (_, _, Some(at), _) => ending_at(raw, span.end + at),
        (_, Some(last), None, _) => *last,
        (_, None, None, Some(at)) => ending_at(raw, at),
        (_, None, None, None) => LineEnding::Lf,
    }
}

/// The ending of the line break (`\n`) at byte `newline` of `raw`.
fn ending_at(raw: &str, newline: usize) -> LineEnding {
    match raw.get(..newline) {
        Some(before) if before.ends_with('\r') => LineEnding::Crlf,
        Some(_) | None => LineEnding::Lf,
    }
}

/// The endings of `text`'s line breaks, in order: `\r\n` or `\n` (a lone
/// `\r` starts no line, as `read` shows it).
fn line_endings(text: &str) -> Vec<LineEnding> {
    text.match_indices('\n')
        .map(|(at, _)| ending_at(text, at))
        .collect()
}

/// `lf` (normalised text: its only line break is `\n`) with each line
/// break written as the one it replaces (#2242 review 2): the n-th break of
/// `lf` takes the ending of the n-th line break of `span` (the file's text
/// the edit replaces), and breaks beyond the span's own take `beyond`. So a
/// replaced line keeps its own ending, even in a span of mixed endings.
pub(super) fn with_span_endings(lf: &str, span: &str, beyond: LineEnding) -> String {
    debug_assert!(!lf.contains('\r'), "{lf:?} is not normalised");
    let mut endings = line_endings(span).into_iter();
    let mut out = String::with_capacity(lf.len() + lf.len() / 8);
    for (index, line) in lf.split('\n').enumerate() {
        if index > 0 {
            match endings.next().unwrap_or(beyond) {
                LineEnding::Crlf => out.push_str("\r\n"),
                LineEnding::Lf => out.push('\n'),
            }
        }
        out.push_str(line);
    }
    debug_assert_eq!(out.replace("\r\n", "\n"), lf);
    out
}

/// Replace `range` of `text` with `new`, or `None` when the range is not on
/// character boundaries of `text`. Bytes outside `range` are kept.
pub(super) fn splice(text: &str, range: std::ops::Range<usize>, new: &str) -> Option<String> {
    let range = (range.start <= range.end).then_some(range)?;
    let before = text.get(..range.start)?;
    let after = text.get(range.end..)?;
    let mut out = String::with_capacity(before.len() + new.len() + after.len());
    out.push_str(before);
    out.push_str(new);
    out.push_str(after);
    debug_assert!(out.starts_with(before) && out.ends_with(after));
    Some(out)
}

/// The file's text as the success diff shows it: its lines as `read` shows
/// them. The BOM is dropped and each `\r\n` is shown as `\n`; a lone `\r`
/// stays in its line, since it starts no line in the file.
pub(super) fn line_view(raw: &str) -> String {
    let body = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    body.replace("\r\n", "\n")
}

/// The offset in [`line_view`] of byte `at` of `raw`: less the BOM and one
/// byte for each `\r\n` before it. `at` is never inside a `\r\n` or the
/// BOM: it is where a character of the file starts or ends.
pub(super) fn view_offset(raw: &str, at: usize) -> usize {
    let bom = raw.len() - raw.strip_prefix('\u{FEFF}').unwrap_or(raw).len();
    let before = raw.get(bom..at);
    debug_assert!(
        before.is_some(),
        "{at} is not a character offset of the text"
    );
    let before = before.unwrap_or_default();
    debug_assert!(
        !(before.ends_with('\r') && raw.get(at..).is_some_and(|rest| rest.starts_with('\n'))),
        "{at} is inside a \\r\\n"
    );
    before.len() - before.matches("\r\n").count()
}
