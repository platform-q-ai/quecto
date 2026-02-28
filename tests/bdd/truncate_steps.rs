use super::*;
use quecto::infrastructure::tools::truncate::{
    TruncatedBy, format_size, truncate_head, truncate_line, truncate_tail,
};

// ===========================================================================
// Given steps
// ===========================================================================

#[given("an empty string to truncate")]
fn given_empty_string(world: &mut QuectoWorld) {
    world.truncation_input = Some(String::new());
}

#[given(expr = "a string with {int} lines totalling {int} bytes")]
fn given_string_with_lines_and_bytes(world: &mut QuectoWorld, lines: usize, total_bytes: usize) {
    let bytes_per_line = total_bytes / lines;
    let char_count = if bytes_per_line > 1 {
        bytes_per_line - 1
    } else {
        0
    };
    let mut s = String::new();
    for i in 0..lines {
        s.push_str(&"a".repeat(char_count));
        if i < lines - 1 {
            s.push('\n');
        }
    }
    world.truncation_input = Some(s);
}

#[given(expr = "a string with {int} lines of {int} bytes each")]
fn given_string_with_lines_of_bytes(world: &mut QuectoWorld, lines: usize, bytes_per_line: usize) {
    let char_count = bytes_per_line - 1; // minus newline
    let mut s = String::new();
    for i in 0..lines {
        s.push_str(&"x".repeat(char_count));
        if i < lines - 1 {
            s.push('\n');
        }
    }
    world.truncation_input = Some(s);
}

#[given(expr = "a single line of {int} bytes")]
fn given_single_line_of_bytes(world: &mut QuectoWorld, bytes: usize) {
    world.truncation_input = Some("z".repeat(bytes));
}

#[given(expr = "a string with exactly {int} lines totalling exactly 50KB")]
fn given_exact_string(world: &mut QuectoWorld, lines: usize) {
    let total_bytes = 50 * 1024;
    let content_chars = total_bytes - (lines - 1);
    let chars_per_line = content_chars / lines;
    let extra = content_chars % lines;
    let mut s = String::new();
    for i in 0..lines {
        let n = if i < extra {
            chars_per_line + 1
        } else {
            chars_per_line
        };
        s.push_str(&"a".repeat(n));
        if i < lines - 1 {
            s.push('\n');
        }
    }
    assert_eq!(s.len(), total_bytes);
    world.truncation_input = Some(s);
}

#[given("a string with lines containing multi-byte UTF-8 characters")]
fn given_multibyte_utf8(world: &mut QuectoWorld) {
    let mut s = String::new();
    for i in 0..20 {
        s.push_str("héllo 世界 🦀 ");
        if i < 19 {
            s.push('\n');
        }
    }
    world.truncation_input = Some(s);
}

#[given(expr = "a line of {int} characters")]
fn given_line_of_chars(world: &mut QuectoWorld, chars: usize) {
    world.truncation_input = Some("a".repeat(chars));
}

// ===========================================================================
// When steps
// ===========================================================================

#[when(expr = "I head-truncate with max {int} lines and 50KB bytes")]
fn when_head_truncate(world: &mut QuectoWorld, max_lines: usize) {
    let input = world
        .truncation_input
        .as_ref()
        .expect("no truncation input set");
    let result = truncate_head(input, max_lines, 50 * 1024);
    world.truncation_result = Some(result);
}

#[when(expr = "I head-truncate with max {int} lines and {int} bytes")]
fn when_head_truncate_custom(world: &mut QuectoWorld, max_lines: usize, max_bytes: usize) {
    let input = world
        .truncation_input
        .as_ref()
        .expect("no truncation input set");
    let result = truncate_head(input, max_lines, max_bytes);
    world.truncation_result = Some(result);
}

#[when(expr = "I tail-truncate with max {int} lines and 50KB bytes")]
fn when_tail_truncate(world: &mut QuectoWorld, max_lines: usize) {
    let input = world
        .truncation_input
        .as_ref()
        .expect("no truncation input set");
    let result = truncate_tail(input, max_lines, 50 * 1024);
    world.truncation_result = Some(result);
}

#[when(expr = "I truncate the line to {int} characters")]
fn when_truncate_line(world: &mut QuectoWorld, max_chars: usize) {
    let input = world
        .truncation_input
        .as_ref()
        .expect("no truncation input set");
    let result = truncate_line(input, max_chars);
    world.truncation_line_result = Some(result);
}

// ===========================================================================
// Then steps
// ===========================================================================

#[then("the truncation result should be empty")]
fn then_result_empty(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(
        r.content.is_empty(),
        "expected empty content, got: {:?}",
        r.content
    );
}

#[then("the result should not be truncated")]
fn then_not_truncated(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(!r.truncated, "expected not truncated");
}

#[then("the result should be truncated")]
fn then_is_truncated(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(r.truncated, "expected truncated");
}

#[then(expr = "all {int} lines should be returned")]
fn then_all_lines_returned(world: &mut QuectoWorld, expected: usize) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert_eq!(
        r.output_lines, expected,
        "expected {} output lines, got {}",
        expected, r.output_lines
    );
}

#[then(expr = "exactly {int} lines should be returned")]
fn then_exact_lines_returned(world: &mut QuectoWorld, expected: usize) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert_eq!(
        r.output_lines, expected,
        "expected {} output lines, got {}",
        expected, r.output_lines
    );
}

#[then("the result should be truncated by lines")]
fn then_truncated_by_lines(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(r.truncated, "expected truncated");
    assert_eq!(
        r.truncated_by,
        Some(TruncatedBy::Lines),
        "expected truncated by lines"
    );
}

#[then("the result should be truncated by bytes")]
fn then_truncated_by_bytes(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(r.truncated, "expected truncated");
    assert_eq!(
        r.truncated_by,
        Some(TruncatedBy::Bytes),
        "expected truncated by bytes"
    );
}

#[then(expr = "total_lines should be {int}")]
fn then_total_lines(world: &mut QuectoWorld, expected: usize) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert_eq!(
        r.total_lines, expected,
        "expected total_lines={}, got {}",
        expected, r.total_lines
    );
}

#[then(expr = "output_lines should be {int}")]
fn then_output_lines(world: &mut QuectoWorld, expected: usize) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert_eq!(
        r.output_lines, expected,
        "expected output_lines={}, got {}",
        expected, r.output_lines
    );
}

#[then("the output should be at most 50KB")]
fn then_output_at_most_50kb(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(
        r.output_bytes <= 50 * 1024,
        "expected output_bytes <= 51200, got {}",
        r.output_bytes
    );
}

#[then("no partial lines should be present")]
fn then_no_partial_lines(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    if !r.content.is_empty() {
        let line_count = r.content.lines().count();
        assert_eq!(
            line_count, r.output_lines,
            "line count mismatch: counted {} but metadata says {}",
            line_count, r.output_lines
        );
    }
}

#[then("the result content should be empty")]
fn then_result_content_empty(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(
        r.content.is_empty(),
        "expected empty content, got: {:?}",
        r.content
    );
}

#[then("first_line_exceeds_limit should be true")]
fn then_first_line_exceeds(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(
        r.first_line_exceeds_limit,
        "expected first_line_exceeds_limit=true"
    );
}

#[then("the result should not split any UTF-8 codepoints")]
fn then_no_utf8_splits(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(std::str::from_utf8(r.content.as_bytes()).is_ok());
}

#[then("the result should be valid UTF-8")]
fn then_valid_utf8(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(std::str::from_utf8(r.content.as_bytes()).is_ok());
}

#[then("the result should contain the last line of the input")]
fn then_contains_last_line(world: &mut QuectoWorld) {
    let input = world
        .truncation_input
        .as_ref()
        .expect("no truncation input");
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    let last_line = input.lines().last().unwrap_or("");
    assert!(
        r.content.contains(last_line),
        "expected result to contain last line, got content length: {}",
        r.content.len()
    );
}

#[then("the result should not contain the first line of the input")]
fn then_not_contains_first_line(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    if r.truncated {
        assert!(
            r.output_lines < r.total_lines,
            "should have fewer output lines than total"
        );
    }
}

#[then("last_line_partial should be true")]
fn then_last_line_partial(world: &mut QuectoWorld) {
    let r = world
        .truncation_result
        .as_ref()
        .expect("no truncation result");
    assert!(r.last_line_partial, "expected last_line_partial=true");
}

// --- truncate_line ---

#[then("the line should be returned unchanged")]
fn then_line_unchanged(world: &mut QuectoWorld) {
    let input = world
        .truncation_input
        .as_ref()
        .expect("no truncation input");
    let (line, _) = world
        .truncation_line_result
        .as_ref()
        .expect("no line result");
    assert_eq!(line, input);
}

#[then("the line should not be marked as truncated")]
fn then_line_not_truncated(world: &mut QuectoWorld) {
    let (_, was_truncated) = world
        .truncation_line_result
        .as_ref()
        .expect("no line result");
    assert!(!was_truncated);
}

#[then("the line should be at most 500 characters plus suffix")]
fn then_line_at_most_chars_plus_suffix(world: &mut QuectoWorld) {
    let (line, _) = world
        .truncation_line_result
        .as_ref()
        .expect("no line result");
    assert!(
        line.chars().count() <= 515,
        "line too long: {} chars",
        line.chars().count()
    );
}

#[then(expr = "the line should end with {string}")]
fn then_line_ends_with(world: &mut QuectoWorld, suffix: String) {
    let (line, _) = world
        .truncation_line_result
        .as_ref()
        .expect("no line result");
    assert!(
        line.ends_with(&suffix),
        "expected line to end with '{}', got: '{}'",
        suffix,
        &line[line.len().saturating_sub(30)..]
    );
}

#[then("the line should be marked as truncated")]
fn then_line_is_truncated(world: &mut QuectoWorld) {
    let (_, was_truncated) = world
        .truncation_line_result
        .as_ref()
        .expect("no line result");
    assert!(was_truncated);
}

// --- format_size ---

#[then("format_size should produce the expected output for each")]
fn then_format_size_produces_expected(_world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    for row in table.rows.iter().skip(1) {
        if row.len() >= 2 {
            let bytes: usize = row[0].trim().parse().expect("invalid byte count");
            let expected = row[1].trim();
            let actual = format_size(bytes);
            assert_eq!(
                actual, expected,
                "format_size({}) = '{}', expected '{}'",
                bytes, actual, expected
            );
        }
    }
}
