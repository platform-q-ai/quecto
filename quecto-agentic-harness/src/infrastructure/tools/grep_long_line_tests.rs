//! #2201: a matching line longer than the line budget is shown around its
//! matches, with where they are, however far into the line they lie.
use super::*;
use tempfile::TempDir;

/// One range, as the hits on a line.
fn one(hit: Range<usize>) -> Vec<Range<usize>> {
    vec![hit]
}

fn grep_in(files: &[(&str, &str)]) -> (GrepTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    for (name, content) in files {
        std::fs::write(tmp.path().join(name), content).unwrap();
    }
    let ws = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (GrepTool::new(ws, sandbox), tmp)
}

async fn search(tool: &GrepTool, args: serde_json::Value) -> String {
    let result = tool.execute(&args.to_string()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    result.content
}

/// The issue's case: `needle` between two 1 MB runs of `a` on one line.
#[tokio::test]
async fn a_match_deep_in_a_two_megabyte_line_is_shown_with_its_offset() {
    let line = format!("{}needle{}\n", "a".repeat(1 << 20), "a".repeat(1 << 20));
    let (tool, _tmp) = grep_in(&[("oneline.txt", &line)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "needle", "path": "oneline.txt"}),
    )
    .await;
    assert!(out.contains("aneedlea"), "the match is shown: {out}");
    assert!(
        out.contains("oneline.txt:1: …[1048329 bytes]…"),
        "the text before it is counted: {out}"
    );
    assert!(
        out.contains("[line is 2.0MB; match at byte 1048576]"),
        "{out}"
    );
    assert!(
        out.contains(
            "[Lines over 500 bytes are cut to that: a matching line around its matches, any \
             other from its start; …[N bytes]… marks text left out. To see more of a line, use \
             bash, e.g. sed -n '<line>p' <file> | cut -b <from>-<to>"
        ),
        "the notice says how lines are cut and how to see more: {out}"
    );
    assert!(!out.contains("read tool"), "{out}");
    assert!(
        out.len() < 2000,
        "the output stays small: {} bytes",
        out.len()
    );
}

/// A context line has no match to show: it is shown from its start.
#[tokio::test]
async fn a_long_context_line_is_shown_from_its_start() {
    let text = format!("{}\nneedle here\n", "c".repeat(900));
    let (tool, _tmp) = grep_in(&[("ctx.txt", &text)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "needle", "path": "ctx.txt", "context": 1}),
    )
    .await;
    let expected = format!("ctx.txt-1- {}…[400 bytes]… [line is 900B]", "c".repeat(500));
    assert!(out.contains(&expected), "{out}");
    assert!(out.contains("ctx.txt:2: needle here"), "{out}");
}

/// Every match on a long line is located; distant ones get windows of
/// their own.
#[tokio::test]
async fn distant_matches_on_one_long_line_each_get_a_window() {
    let line = format!(
        "{}alpha{}omega{}\n",
        "x".repeat(4000),
        "y".repeat(4000),
        "z".repeat(4000)
    );
    let (tool, _tmp) = grep_in(&[("two.txt", &line)]);
    let out = search(
        &tool,
        serde_json::json!({"patterns": ["alpha", "omega"], "path": "two.txt"}),
    )
    .await;
    assert!(out.contains("xalphay"), "{out}");
    assert!(out.contains("yomegaz"), "{out}");
    assert!(out.contains("2 matches, first at byte 4000]"), "{out}");
}

/// rg's submatch offsets are into the whole block a multiline match spans:
/// they are split onto each line.
#[test]
fn submatch_offsets_are_split_onto_the_lines_they_span() {
    let json = r#"{"type":"match","data":{"path":{"text":"/ws/a.rs"},"lines":{"text":"ab cd\nef gh\nij\n"},"line_number":4,"absolute_offset":0,"submatches":[{"match":{"text":"cd\nef"},"start":3,"end":8},{"match":{"text":"j"},"start":13,"end":14}]}}"#;
    let matches = parse_rg_matches(json);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line_count, 3);
    assert_eq!(matches[0].hits, vec![one(3..5), one(0..2), one(1..2)]);
}

#[test]
fn a_submatch_ending_in_the_line_break_stays_on_its_line() {
    let json = r#"{"type":"match","data":{"path":{"text":"/ws/a.rs"},"lines":{"text":"tail\n"},"line_number":1,"absolute_offset":0,"submatches":[{"match":{"text":"il\n"},"start":2,"end":5}]}}"#;
    let matches = parse_rg_matches(json);
    assert_eq!(matches[0].hits, vec![one(2..4)]);
}

/// Offsets that cannot be on the reported lines are dropped, not trusted.
#[test]
fn impossible_submatch_offsets_are_dropped() {
    let json = r#"{"type":"match","data":{"path":{"text":"/ws/a.rs"},"lines":{"text":"short\n"},"line_number":1,"absolute_offset":0,"submatches":[{"match":{"text":"x"},"start":40,"end":41},{"match":{"text":"x"},"start":3,"end":2},{"match":{"text":"s"},"start":0,"end":1}]}}"#;
    let matches = parse_rg_matches(json);
    assert_eq!(matches[0].hits, vec![one(0..1)]);
}

/// Lines rg reports as base64 `bytes` (not UTF-8) are located too.
#[test]
fn submatches_on_undecodable_lines_are_located() {
    use base64::Engine as _;
    let raw = b"\xff\xfe needle\n";
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
    let json = format!(
        r#"{{"type":"match","data":{{"path":{{"text":"/ws/a.bin"}},"lines":{{"bytes":"{encoded}"}},"line_number":1,"absolute_offset":0,"submatches":[{{"match":{{"text":"needle"}},"start":3,"end":9}}]}}}}"#
    );
    let matches = parse_rg_matches(&json);
    assert_eq!(matches[0].hits, vec![one(3..9)]);
    assert_eq!(matches[0].line_count, 1);
}

/// A block's own match is shown around its hits even when the map of
/// matched lines does not list it.
#[test]
fn a_blocks_own_long_line_is_shown_around_its_hits() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("own.txt");
    let line = format!("{}needle{}", "a".repeat(900), "b".repeat(900));
    std::fs::write(&file, format!("{line}\n")).unwrap();
    let ws = tmp.path().to_string_lossy().to_string();
    let prefix = format!("{ws}/");
    let cfg = BlockConfig {
        ws_str: &ws,
        ws_prefix_slash: &prefix,
        context_lines: 0,
        max_line_bytes: MAX_LINE_BYTES,
        max_output_bytes: MAX_OUTPUT_BYTES,
        matched: &HashMap::new(),
    };
    let mut state = FormatState {
        output_lines: vec![],
        byte_total: 0,
        lines_truncated: false,
        truncated_bytes: false,
        shown: HashMap::new(),
    };
    let m = RgMatch {
        file_path: file,
        line_number: 1,
        line_count: 1,
        score: None,
        column: None,
        text: Some(vec![line]),
        hits: vec![one(900..906)],
    };
    assert!(format_match_block(
        &m,
        &mut HashMap::new(),
        &cfg,
        &mut state
    ));
    assert_eq!(state.output_lines.len(), 1, "{:?}", state.output_lines);
    let shown = &state.output_lines[0];
    assert!(shown.starts_with("own.txt:1: …["), "{shown}");
    assert!(shown.contains("aneedleb"), "{shown}");
    assert!(shown.ends_with("; match at byte 900]"), "{shown}");
    assert!(state.lines_truncated);
}
