//! #2163: what the grep tool shows of a content search: every matching line's
//! own text, each line once, and as many matches as asked for.
use super::*;
use tempfile::TempDir;

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

/// A matching line past the first 1 MB of its file is shown with its text:
/// rg reports it; the file cache is only for context.
#[tokio::test]
async fn a_match_past_the_first_megabyte_shows_its_text() {
    let big: String = (0..50_000)
        .map(|i| format!("line {i} {}\n", "x".repeat(80)))
        .collect();
    let (tool, _tmp) = grep_in(&[("big.txt", &big)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "line 4999[0-9] ", "path": "big.txt"}),
    )
    .await;
    assert!(!out.contains("line not shown"), "{out}");
    assert!(out.contains("big.txt:49991: line 49990 x"), "{out}");
    assert!(out.contains("big.txt:50000: line 49999 x"), "{out}");
}

/// Nearby matches share their context: every line appears once, in order,
/// and a matching line is shown as a match, never as another's context.
#[tokio::test]
async fn nearby_matches_share_their_context() {
    let (tool, _tmp) = grep_in(&[("dup.txt", "same\nsame\nunique\n")]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "same", "path": "dup.txt", "context": 1}),
    )
    .await;
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("dup.txt")).collect();
    assert_eq!(
        lines,
        ["dup.txt:1: same", "dup.txt:2: same", "dup.txt-3- unique"],
        "{out}"
    );
}

/// Separate blocks stay separate and each line appears once.
#[tokio::test]
async fn distant_matches_keep_separate_blocks() {
    let text: String = (1..=12)
        .map(|i| format!("row {i}{}\n", if i == 2 || i == 10 { " hit" } else { "" }))
        .collect();
    let (tool, _tmp) = grep_in(&[("rows.txt", &text)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "hit", "path": "rows.txt", "context": 1}),
    )
    .await;
    let numbers: Vec<usize> = out
        .lines()
        .filter_map(|l| l.strip_prefix("rows.txt"))
        .filter_map(|rest| {
            rest[1..]
                .split(|c| c == ':' || c == '-')
                .next()?
                .parse()
                .ok()
        })
        .collect();
    assert_eq!(numbers, [1, 2, 3, 9, 10, 11], "{out}");
}

/// Lines with many hits do not use up the read budget in rg's JSON: the
/// search shows as many matches as asked for.
#[tokio::test]
async fn many_hits_per_line_still_show_the_matches_asked_for() {
    let text: String = (0..1_000)
        .map(|i| format!("{i:04} {}\n", "x".repeat(80)))
        .collect();
    let (tool, _tmp) = grep_in(&[("xs.txt", &text)]);
    let out = search(&tool, serde_json::json!({"pattern": "x", "path": "xs.txt"})).await;
    let shown = out.lines().filter(|l| l.starts_with("xs.txt:")).count();
    assert_eq!(shown, 100, "{out}");
    assert!(!out.contains("rg printed more than"), "{out}");
    assert!(out.contains("100 matches limit reached"), "{out}");
}

/// A ranked match past the first megabyte is judged on its own line, as rg
/// reported it, rather than left unjudged.
#[tokio::test]
async fn a_ranked_match_past_the_first_megabyte_is_judged() {
    use super::rank_by_tests::KeywordJudge;
    let big: String = (0..50_000)
        .map(|i| format!("line {i} {}\n", "x".repeat(80)))
        .collect();
    let (tool, _tmp) = grep_in(&[("big.txt", &big)]);
    let tool = tool.with_relevance(Arc::new(KeywordJudge("line 49990 ")), 100);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "line 4999[0-9] ", "path": "big.txt", "rank_by": "line 49990"}),
    )
    .await;
    assert!(!out.contains("could not be judged"), "{out}");
    let first = out
        .lines()
        .find(|l| l.contains("big.txt:"))
        .unwrap_or_default();
    assert!(
        first.starts_with("[0.95] big.txt:49991: line 49990"),
        "{out}"
    );
}
