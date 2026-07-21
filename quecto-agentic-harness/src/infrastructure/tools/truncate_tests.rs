use super::*;

// -----------------------------------------------------------------------
// truncate_head tests
// -----------------------------------------------------------------------

#[test]
fn head_empty_string() {
    let r = truncate_head("", 2000, 50 * 1024);
    assert!(!r.truncated);
    assert!(r.content.is_empty());
    assert_eq!(r.total_lines, 0);
    assert_eq!(r.output_lines, 0);
}

#[test]
fn head_single_short_line() {
    let r = truncate_head("hello", 2000, 50 * 1024);
    assert!(!r.truncated);
    assert_eq!(r.content, "hello");
    assert_eq!(r.output_lines, 1);
}

#[test]
fn head_truncates_by_line_limit() {
    let mut input = String::new();
    for i in 0..3000 {
        if i > 0 {
            input.push('\n');
        }
        input.push_str(&format!("line{i}"));
    }
    let r = truncate_head(&input, 2000, 50 * 1024 * 1024); // large byte limit
    assert!(r.truncated);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
    assert_eq!(r.output_lines, 2000);
    assert_eq!(r.total_lines, 3000);
}

#[test]
fn head_truncates_by_byte_limit() {
    // 100 lines of ~1000 bytes each = ~100KB > 50KB
    let line = "x".repeat(999);
    let input: String = (0..100)
        .map(|_| line.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let r = truncate_head(&input, 2000, 50 * 1024);
    assert!(r.truncated);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Bytes));
    assert!(r.output_bytes <= 50 * 1024);
}

#[test]
fn head_first_line_exceeds_limit() {
    let input = "z".repeat(60000);
    let r = truncate_head(&input, 2000, 50 * 1024);
    assert!(r.truncated);
    assert!(r.first_line_exceeds_limit);
    assert!(r.content.is_empty());
}

#[test]
fn head_exact_at_limit() {
    // Build exactly 2000 lines totalling exactly 50KB
    let total_bytes = 50 * 1024;
    let lines = 2000;
    let newlines = lines - 1;
    let content_chars = total_bytes - newlines;
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
    let r = truncate_head(&s, 2000, 50 * 1024);
    assert!(!r.truncated);
    assert_eq!(r.output_lines, 2000);
}

#[test]
fn head_multibyte_utf8_keeps_whole_characters() {
    let input = "éé\nline2";
    let r = truncate_head(input, 2000, 5);
    assert_eq!(r.content, "éé");
    assert_eq!(r.output_bytes, 4);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Bytes));
}

#[test]
fn head_multibyte_utf8_large_input_never_splits_codepoints() {
    // Wide-input stress case: mixed 2/3/4-byte characters where the byte
    // cap lands mid-codepoint on many candidate cut points.
    let input: String = (0..20)
        .map(|_| "héllo 世界 🦀")
        .collect::<Vec<_>>()
        .join("\n");
    let r = truncate_head(&input, 10, 100);
    assert!(r.truncated);
    assert!(r.output_bytes <= 100);
    // The kept prefix must cut the input at a character boundary and be
    // byte-identical to the original up to that point.
    assert!(input.is_char_boundary(r.content.len()));
    assert_eq!(r.content, input[..r.content.len()]);
}

#[test]
fn head_no_partial_lines() {
    let line = "x".repeat(999);
    let input: String = (0..100)
        .map(|_| line.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let r = truncate_head(&input, 2000, 50 * 1024);
    let counted = r.content.lines().count();
    assert_eq!(counted, r.output_lines);
}

// -----------------------------------------------------------------------
// truncate_tail tests
// -----------------------------------------------------------------------

#[test]
fn tail_empty_string() {
    let r = truncate_tail("", 2000, 50 * 1024);
    assert!(!r.truncated);
    assert!(r.content.is_empty());
}

#[test]
fn tail_single_short_line() {
    let r = truncate_tail("hello", 2000, 50 * 1024);
    assert!(!r.truncated);
    assert_eq!(r.content, "hello");
}

#[test]
fn tail_truncates_by_line_limit_keeps_last() {
    let mut input = String::new();
    for i in 0..3000 {
        if i > 0 {
            input.push('\n');
        }
        input.push_str(&format!("line{i}"));
    }
    let r = truncate_tail(&input, 2000, 50 * 1024 * 1024);
    assert!(r.truncated);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
    assert_eq!(r.output_lines, 2000);
    assert!(r.content.contains("line2999"));
    assert!(!r.content.starts_with("line0\n"));
}

#[test]
fn tail_truncates_by_byte_limit_keeps_last() {
    let line = "y".repeat(999);
    let input: String = (0..100)
        .map(|_| line.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let r = truncate_tail(&input, 2000, 50 * 1024);
    assert!(r.truncated);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Bytes));
    assert!(r.output_bytes <= 50 * 1024);
    // Should contain the last line
    assert!(r.content.ends_with(&line));
}

#[test]
fn tail_truncation_keeps_requested_last_lines_for_large_content() {
    let mut input = String::new();
    for i in 1..=5000 {
        input.push_str(&format!("line{i}\n"));
    }
    let r = truncate_tail(&input, 3, 1024);
    assert_eq!(r.content, "line4998\nline4999\nline5000");
    assert_eq!(r.output_lines, 3);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Lines));
}

#[test]
fn tail_single_huge_line_partial() {
    let input = "z".repeat(60000);
    let r = truncate_tail(&input, 2000, 50 * 1024);
    assert!(r.truncated);
    assert!(r.last_line_partial);
    assert!(r.output_bytes <= 50 * 1024);
    // Content should be the tail of the input
    assert!(input.ends_with(&r.content));
}

#[test]
fn tail_strips_carriage_return_before_newline_like_lines() {
    // "\r\n"-terminated lines must not count the '\r' toward the byte
    // budget — `str::lines()` treats "\r\n" as one terminator.
    let r = truncate_tail("aa\r\nbb\r\ncc", 2000, 8);
    assert_eq!(r.content, "aa\nbb\ncc");
    assert!(!r.truncated);
}

#[test]
fn tail_counts_trailing_carriage_return_toward_byte_limit() {
    let input = format!("{}\r", "z".repeat(50 * 1024));
    let r = truncate_tail(&input, 2000, 50 * 1024);
    assert!(r.truncated);
    assert!(r.last_line_partial);
    assert!(r.output_bytes <= 50 * 1024);
    assert_eq!(r.content.len(), 50 * 1024);
}

#[test]
fn tail_utf8_safe_keeps_whole_characters() {
    let input = "a\nééé";
    let r = truncate_tail(input, 2000, 5);
    assert_eq!(r.content, "éé");
    assert_eq!(r.output_bytes, 4);
    assert!(r.last_line_partial);
}

#[test]
fn tail_utf8_safe_large_multibyte_input() {
    // Wide-input stress case: 80KB of 4-byte chars on a single line, so
    // the byte cap falls mid-codepoint unless the boundary is adjusted.
    let input = "🦀".repeat(20000);
    let r = truncate_tail(&input, 2000, 50 * 1024);
    assert!(r.truncated);
    assert!(r.last_line_partial);
    assert!(r.output_bytes <= 50 * 1024);
    // The kept suffix must be whole characters and match the input tail.
    assert!(r.content.chars().all(|c| c == '🦀'));
    assert!(input.ends_with(&r.content));
}

// -----------------------------------------------------------------------
// truncate_line tests
// -----------------------------------------------------------------------

#[test]
fn line_short_unchanged() {
    let (result, truncated) = truncate_line("hello world", 500);
    assert_eq!(result, "hello world");
    assert!(!truncated);
}

#[test]
fn line_exact_at_limit() {
    let input = "a".repeat(500);
    let (result, truncated) = truncate_line(&input, 500);
    assert_eq!(result, input);
    assert!(!truncated);
}

#[test]
fn line_over_limit_truncated() {
    let input = "a".repeat(600);
    let (result, truncated) = truncate_line(&input, 500);
    assert!(truncated);
    assert!(result.ends_with("... [truncated]"));
    assert!(result.chars().count() <= 515); // 500 + "... [truncated]"
}

// -----------------------------------------------------------------------
// format_size tests
// -----------------------------------------------------------------------

#[test]
fn format_size_bytes() {
    assert_eq!(format_size(0), "0B");
    assert_eq!(format_size(512), "512B");
    assert_eq!(format_size(1023), "1023B");
}

#[test]
fn format_size_kilobytes() {
    assert_eq!(format_size(1024), "1.0KB");
    assert_eq!(format_size(1536), "1.5KB");
    assert_eq!(format_size(51200), "50.0KB");
}

#[test]
fn format_size_megabytes() {
    assert_eq!(format_size(1048576), "1.0MB");
    assert_eq!(format_size(1572864), "1.5MB");
}
