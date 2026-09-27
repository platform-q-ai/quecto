//! Over-long lines of `bash` output over the 50KB budget (#2196). There a
//! single line of minified code, base64 or a long log record would fill
//! the tail (~12k tokens) and show as a whole line when it is not. Each line longer than
//! [`LINE_MAX_BYTES`] keeps its first and last [`LINE_KEEP_BYTES`], with the
//! middle named in place as the capture names a dropped middle
//! (`[... N bytes of line L omitted ...]`). The saved output keeps the
//! whole line.
use std::borrow::Cow;

/// A line up to this long is shown whole.
pub(super) const LINE_MAX_BYTES: usize = 8 * 1024;
/// A longer line shows this much of its start, and as much of its end.
pub(super) const LINE_KEEP_BYTES: usize = 2 * 1024;

/// A line shown as its start and end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CutLine {
    /// Its 1-based number, counted as the tail counts lines.
    pub number: usize,
    /// Its length in bytes, without its terminator.
    pub bytes: usize,
}

/// The output with every over-long line cut to its window.
pub(super) struct Windowed<'a> {
    pub text: Cow<'a, str>,
    /// The cut lines, in order.
    pub cut: Vec<CutLine>,
}

/// Cut each line longer than [`LINE_MAX_BYTES`] to its first and last
/// [`LINE_KEEP_BYTES`]. Lines are split as `str::lines` splits them (a
/// `\r` before `\n` belongs to the terminator), so numbering and line count
/// are unchanged; terminators are kept byte for byte.
pub(super) fn window_long_lines(text: &str) -> Windowed<'_> {
    const { assert!(2 * LINE_KEEP_BYTES < LINE_MAX_BYTES) };
    let fits = |line: &str| line_body(line).len() <= LINE_MAX_BYTES;
    if text.split_inclusive('\n').all(fits) {
        return Windowed {
            text: Cow::Borrowed(text),
            cut: Vec::new(),
        };
    }
    let mut out = String::with_capacity(text.len().min(64 * 1024));
    let mut cut = Vec::new();
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let body = line_body(line);
        let terminator = &line[body.len()..];
        match body.len() > LINE_MAX_BYTES {
            true => {
                let number = index + 1;
                push_window(&mut out, body, number);
                cut.push(CutLine {
                    number,
                    bytes: body.len(),
                });
            }
            false => out.push_str(body),
        }
        out.push_str(terminator);
    }
    debug_assert_eq!(out.lines().count(), text.lines().count());
    Windowed {
        text: Cow::Owned(out),
        cut,
    }
}

/// A line without its `\n` or `\r\n` terminator.
fn line_body(line: &str) -> &str {
    match line.strip_suffix('\n') {
        Some(body) => body.strip_suffix('\r').unwrap_or(body),
        None => line,
    }
}

/// The first and last [`LINE_KEEP_BYTES`] of `body`, cut on character
/// boundaries, with the omitted middle named between them.
fn push_window(out: &mut String, body: &str, number: usize) {
    assert!(body.len() > LINE_MAX_BYTES, "only an over-long line is cut");
    // `str::floor_char_boundary` is past the MSRV: a character is at most
    // four bytes, so a boundary is at most three steps away.
    let head_end = (0..=LINE_KEEP_BYTES)
        .rev()
        .find(|at| body.is_char_boundary(*at))
        .unwrap_or(0);
    let tail_start = (body.len() - LINE_KEEP_BYTES..=body.len())
        .find(|at| body.is_char_boundary(*at))
        .unwrap_or(body.len());
    assert!(head_end < tail_start, "the window drops a middle");
    let omitted = tail_start - head_end;
    out.push_str(&body[..head_end]);
    out.push_str(&format!(
        "[... {omitted} bytes of line {number} omitted ...]"
    ));
    out.push_str(&body[tail_start..]);
}

/// The note after the output about the cut lines it shows (`cut` is not
/// empty).
pub(super) fn cut_note(cut: &[CutLine]) -> String {
    const { assert!(LINE_KEEP_BYTES % 1024 == 0 && LINE_MAX_BYTES % 1024 == 0) };
    let keep = format!("{}KB", LINE_KEEP_BYTES / 1024);
    match cut {
        [] => {
            debug_assert!(false, "a note needs a cut line");
            String::new()
        }
        [only] => format!(
            "line {} is {} bytes, of which the first and last {keep} are shown",
            only.number, only.bytes
        ),
        many => {
            let longest = many.iter().map(|line| line.bytes).max().unwrap_or(0);
            let cap = format!("{}KB", LINE_MAX_BYTES / 1024);
            format!(
                "{} lines over {cap} (the longest {longest} bytes) show only their first and \
                 last {keep}",
                many.len()
            )
        }
    }
}

#[cfg(test)]
#[path = "long_lines_tests.rs"]
mod tests;
