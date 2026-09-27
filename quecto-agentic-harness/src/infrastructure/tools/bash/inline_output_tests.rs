//! #2196: output within the 50KB budget keeps every line whole; output over
//! it shows each over-long line as its start and end, says what it shows —
//! lines and bytes — and how to see the rest, and costs a few KB, not 50.
use super::truncate_output;
use crate::infrastructure::tools::bash::long_lines::{LINE_KEEP_BYTES, LINE_MAX_BYTES};

/// The saved path in a note: the text after "saved to: ", to the `]`.
fn saved_path(out: &str) -> String {
    out.rsplit("saved to: ")
        .next()
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or_else(|| panic!("no saved path in {out}"))
        .to_string()
}

/// One-line JSON of 15KB, as `gh api` or `jq -c` prints it.
fn one_line_json(bytes: usize) -> String {
    let mut json = String::from("{\"items\":[");
    while json.len() + 40 < bytes {
        json.push_str("{\"id\":12345,\"name\":\"an item name\"},");
    }
    json.push_str("{\"id\":0}]}");
    json
}

#[tokio::test]
async fn one_line_json_within_the_budget_comes_back_whole() {
    let json = one_line_json(15 * 1024);
    assert!(json.len() > LINE_MAX_BYTES);
    assert_eq!(truncate_output(json.clone(), false).await, json);
}

#[tokio::test]
async fn several_long_lines_within_the_budget_come_back_whole() {
    let line = one_line_json(15 * 1024);
    let text = format!("{line}\n{line}\n{line}");
    assert!(text.len() > 45_000 && text.len() <= 50 * 1024);
    assert_eq!(truncate_output(text.clone(), false).await, text);
}

#[tokio::test]
async fn a_single_over_long_line_names_the_part_it_shows() {
    let line = "b".repeat(60_000);
    let out = truncate_output(line.clone(), false).await;
    let (shown, note) = out.split_once("\n[").unwrap();
    assert_eq!(
        shown,
        format!(
            "{}[... {} bytes of line 1 omitted ...]{}",
            "b".repeat(LINE_KEEP_BYTES),
            60_000 - 2 * LINE_KEEP_BYTES,
            "b".repeat(LINE_KEEP_BYTES)
        )
    );
    let path = saved_path(&out);
    assert_eq!(
        note,
        format!(
            "Showing lines 1-1 of 1 (50KB limit); line 1 is 60000 bytes, of which the first and \
             last 2KB are shown. `read` the saved file for lines up to 50KB whole, or reformat \
             the output with `jq .` / `fold -w 200`. Full output (60000 bytes) saved to: {path}]"
        )
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        line,
        "the whole line is saved"
    );
    let _ = std::fs::remove_file(path);
    assert!(out.len() < 5 * 1024, "a few KB, not 50: {}", out.len());
}

/// A 60KB line among other output over the budget is cut; the short lines
/// around it are kept whole.
#[tokio::test]
async fn a_long_line_among_other_output_is_cut() {
    let text = format!("head\n{}\ntail", "c".repeat(60_000));
    let out = truncate_output(text, false).await;
    assert!(out.starts_with("head\nccc"), "{out}");
    assert!(out.contains("bytes of line 2 omitted"), "{out}");
    assert!(
        out.contains("\ntail\n[Showing lines 1-3 of 3 (50KB limit); line 2 is 60000 bytes"),
        "{out}"
    );
    let _ = std::fs::remove_file(saved_path(&out));
}

/// Lines that fit keep the notice they had.
#[tokio::test]
async fn output_of_lines_that_fit_keeps_its_notice() {
    let text: String = (0..1500).map(|i| format!("{i:060}\n")).collect();
    let out = truncate_output(text, false).await;
    let note = out.rsplit_once("\n[").unwrap().1;
    assert!(
        note.starts_with("Showing lines 662-1500 of 1500 (50KB limit). Full output"),
        "{note}"
    );
    assert!(!out.contains("omitted"), "{note}");
    let _ = std::fs::remove_file(saved_path(&out));
}

/// Output cut by line count alone fits the byte budget: a long line in its
/// tail stays whole.
#[tokio::test]
async fn a_long_line_in_a_line_cut_tail_stays_whole() {
    let mut text: String = (0..2100).map(|i| format!("{i}\n")).collect();
    let long = "d".repeat(LINE_MAX_BYTES * 2);
    text.push_str(&long);
    let out = truncate_output(text, false).await;
    assert!(
        out.contains(&format!("\n{long}\n[")),
        "the long line is whole"
    );
    let note = out.rsplit_once("\n[").unwrap().1;
    assert!(
        note.starts_with("Showing lines 102-2101 of 2101. Full output"),
        "{note}"
    );
    let _ = std::fs::remove_file(saved_path(&out));
}

/// A cut line the tail leaves out is not named; one at the tail's first
/// line is.
#[tokio::test]
async fn only_cut_lines_in_the_tail_are_named() {
    let mut text = format!("{}\n", "d".repeat(60_000));
    text.extend((0..60).map(|i| format!("{i:0999}\n")));
    let out = truncate_output(text, false).await;
    let note = out.rsplit_once("\n[").unwrap().1;
    assert!(
        note.starts_with("Showing lines 11-61 of 61 (50KB limit). Full output"),
        "{note}"
    );
    let _ = std::fs::remove_file(saved_path(&out));

    let mut text = format!("{}\n", "e".repeat(60_000));
    text.extend((0..40).map(|i| format!("{i:0999}\n")));
    let out = truncate_output(text, false).await;
    let note = out.rsplit_once("\n[").unwrap().1;
    assert!(
        note.starts_with("Showing lines 1-41 of 41 (50KB limit); line 1 is 60000 bytes"),
        "{note}"
    );
    let _ = std::fs::remove_file(saved_path(&out));
}

/// #2196 review: output over the budget whose windowed tail is cut by line
/// count is not labelled as the 50KB limit's — the reviewer's
/// `python3 -c "print('x'*60000); [print(i) for i in range(2100)]"`.
#[tokio::test]
async fn a_line_count_cut_of_windowed_output_is_not_the_50kb_limit() {
    let mut text = format!("{}\n", "x".repeat(60_000));
    text.extend((0..2100).map(|i| format!("{i}\n")));
    let out = truncate_output(text, false).await;
    let note = out.rsplit_once("\n[").unwrap().1;
    assert!(
        note.starts_with("Showing lines 102-2101 of 2101. Full output"),
        "{note}"
    );
    let _ = std::fs::remove_file(saved_path(&out));

    // A long line in that tail is cut, and named, still without the label.
    let mut text: String = (0..2100).map(|i| format!("{i}\n")).collect();
    text.push_str(&"y".repeat(60_000));
    let out = truncate_output(text, false).await;
    let note = out.rsplit_once("\n[").unwrap().1;
    assert!(
        note.starts_with("Showing lines 102-2101 of 2101; line 2101 is 60000 bytes"),
        "{note}"
    );
    let _ = std::fs::remove_file(saved_path(&out));
}

/// Several cut lines are counted, with the longest named.
#[tokio::test]
async fn several_cut_lines_are_counted() {
    let text = format!(
        "{}\n{}\n{}",
        "e".repeat(LINE_MAX_BYTES + 1),
        "f".repeat(20_000),
        "g".repeat(30_000)
    );
    let out = truncate_output(text, false).await;
    assert!(
        out.contains(
            "Showing lines 1-3 of 3 (50KB limit); 3 lines over 8KB (the longest 30000 bytes)"
        ),
        "{out}"
    );
    let _ = std::fs::remove_file(saved_path(&out));
}

#[tokio::test]
async fn short_output_is_returned_as_it_is() {
    assert_eq!(truncate_output("a\nb\n".into(), false).await, "a\nb");
    assert_eq!(truncate_output(String::new(), false).await, "");
    let at_budget = "g".repeat(50 * 1024);
    assert_eq!(truncate_output(at_budget.clone(), false).await, at_budget);
}

/// #2196, #2197: the description names the per-line cut and binary output,
/// and stays within its byte budget.
#[test]
fn the_description_names_long_lines_and_binary_output_within_budget() {
    use crate::application::tools::ports::Tool;
    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(None);
    let tool = crate::infrastructure::tools::bash::ExecTool::new(
        std::sync::Arc::new(tmp.path().to_path_buf()),
        std::sync::Arc::new(sandbox),
    );
    let description = tool.definition().description;
    assert!(description.contains("over 8KB"), "{description}");
    assert!(description.contains("Binary output"), "{description}");
    assert!(description.len() <= 1200, "{} bytes", description.len());
}
