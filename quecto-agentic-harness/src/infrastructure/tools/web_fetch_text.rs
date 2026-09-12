/// Append `line` to `out` with runs of whitespace collapsed to single spaces,
/// leading/trailing whitespace trimmed. No per-line allocation.
fn push_collapsed_line(out: &mut String, line: &str) {
    let mut prev_space = true; // true = trim leading spaces
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    // Trim trailing space
    if out.ends_with(' ') {
        out.pop();
    }
}

/// Collapse runs of whitespace into single spaces, blank lines into single
/// blank lines, and trim each line. Writes directly into a single output
/// buffer to avoid per-line heap allocations.
pub(super) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(256 * 1024));
    let mut consecutive_blank = 0_u32;
    let mut first_line = true;

    for line in text.lines() {
        if line.chars().all(|c| c.is_whitespace()) {
            consecutive_blank += 1;
            if consecutive_blank <= 1 {
                if !first_line {
                    out.push('\n');
                }
                first_line = false;
            }
            continue;
        }

        consecutive_blank = 0;
        if !first_line {
            out.push('\n');
        }
        first_line = false;
        push_collapsed_line(&mut out, line);
    }

    // Trim leading/trailing blank lines in-place
    while out.ends_with('\n') {
        out.pop();
    }
    if let Some(start) = out.find(|c: char| c != '\n') {
        if start > 0 {
            out.drain(..start);
        }
    }
    out
}
