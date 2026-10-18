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
        .filter_map(|rest| rest[1..].split([':', '-']).next()?.parse().ok())
        .collect();
    assert_eq!(numbers, [1, 2, 3, 9, 10, 11], "{out}");
}

/// Lines with many hits do not use up the read budget in rg's JSON: the
/// search shows as many matches as asked for.
#[tokio::test]
async fn many_hits_per_line_still_show_the_matches_asked_for() {
    // ~18 MB of rg JSON: a byte budget alone would stop long before the end.
    let text: String = (0..5_000)
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

/// A matching line cut part-way by the 1 MB file cache is shown whole, as
/// rg reported it, not as the cache's cut copy.
#[tokio::test]
async fn a_match_straddling_the_cache_boundary_is_shown_whole() {
    let padding = "p".repeat(99) + "\n";
    let mut text = padding.repeat(1024 * 1024 / 100);
    text.push_str(&format!("needle {} END\n", "y".repeat(200)));
    let (tool, _tmp) = grep_in(&[("edge.txt", &text)]);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "needle", "path": "edge.txt"}),
    )
    .await;
    let shown = out
        .lines()
        .find(|l| l.contains("needle"))
        .unwrap_or_default();
    assert!(shown.ends_with("END"), "{out}");
}

/// Review #2174: in rank order a match earlier in a file than the one
/// ranked above it is still shown, and files that interleave repeat no line.
#[tokio::test]
async fn a_better_ranked_later_match_does_not_hide_an_earlier_one() {
    use super::rank_by_tests::KeywordJudge;
    let text: String = (1..=60)
        .map(|i| match i {
            10 => "hit low\n".to_string(),
            50 => "hit best\n".to_string(),
            _ => format!("row {i}\n"),
        })
        .collect();
    let (tool, _tmp) = grep_in(&[("r.txt", &text)]);
    let tool = tool.with_relevance(Arc::new(KeywordJudge("best")), 100);
    for context in [0, 2] {
        let out = search(
            &tool,
            serde_json::json!({"pattern": "hit", "path": "r.txt", "rank_by": "best", "context": context}),
        )
        .await;
        assert!(out.contains("r.txt:50: hit best"), "{out}");
        assert!(
            out.contains("r.txt:10: hit low"),
            "context {context}: {out}"
        );
        let mut shown: Vec<&str> = out.lines().filter(|l| l.contains("r.txt")).collect();
        let all = shown.len();
        shown.sort();
        shown.dedup();
        assert_eq!(shown.len(), all, "a line repeated: {out}");
    }
}

/// Review #2174 (round 2): a match already shown inside a better one's block
/// prints nothing more, so no stray context appears away from its match.
#[tokio::test]
async fn a_match_shown_in_a_better_block_leaves_no_stray_context() {
    use super::rank_by_tests::KeywordJudge;
    let text: String = (1..=60)
        .map(|i| match i {
            10 => "hit mid\n".to_string(),
            48 => "hit low\n".to_string(),
            50 => "hit best\n".to_string(),
            _ => format!("row {i}\n"),
        })
        .collect();
    let (tool, _tmp) = grep_in(&[("r.txt", &text)]);
    let tool = tool.with_relevance(Arc::new(KeywordJudge("best")), 100);
    let out = search(
        &tool,
        serde_json::json!({"pattern": "hit", "path": "r.txt", "rank_by": "best", "context": 2}),
    )
    .await;
    // Best first; the lower match comes later with its context right
    // before it, and no line repeats (#2174 swarm review).
    let lines: Vec<&str> = out.lines().filter(|l| l.contains("r.txt")).collect();
    assert!(
        lines[0].starts_with("r.txt-49-") && lines[1].starts_with("[0.95] r.txt:50:"),
        "{out}"
    );
    let low = lines
        .iter()
        .position(|l| l.contains("r.txt:48: hit low"))
        .expect("48 shown");
    assert!(low > 1, "{out}");
    assert!(
        lines[low - 1].starts_with("r.txt-47-") && lines[low - 2].starts_with("r.txt-46-"),
        "{out}"
    );
    let mut sorted = lines.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), lines.len(), "a line repeated: {out}");
}
