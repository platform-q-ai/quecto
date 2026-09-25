use super::*;
use crate::application::search::ports::PortFuture;
use crate::infrastructure::security::sandbox::Sandbox;
use std::sync::Mutex;

/// A judge answering from a script, remembering what it was shown.
struct ScriptedJudge {
    answer: Relevance,
    seen: Mutex<Vec<(String, Vec<RelevanceCandidate>)>>,
}

impl RelevanceJudge for ScriptedJudge {
    fn judge<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        self.seen
            .lock()
            .unwrap()
            .push((query.to_string(), candidates.to_vec()));
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}

fn ranking(answer: Relevance, max_candidates: usize) -> (Ranking, Arc<ScriptedJudge>) {
    let judge = Arc::new(ScriptedJudge {
        answer,
        seen: Mutex::default(),
    });
    (
        Ranking {
            judge: judge.clone(),
            max_candidates,
        },
        judge,
    )
}

/// Three one-line matches in `src/lib.rs` (lines 2, 4, 6) of a real file,
/// so the judge's text can be read.
fn workspace() -> (tempfile::TempDir, Vec<RgMatch>) {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "a\nfn retry() {}\nb\nlet retry_count = 3;\nc\n// retry later\nd\n",
    )
    .unwrap();
    let matches = [2, 4, 6]
        .into_iter()
        .map(|line_number| RgMatch {
            file_path: dir.path().join("src/lib.rs"),
            line_number,
            line_count: 1,
            score: None,
            column: None,
        })
        .collect();
    (dir, matches)
}

fn lines(matches: &[RgMatch]) -> Vec<(usize, Option<f64>)> {
    matches.iter().map(|m| (m.line_number, m.score)).collect()
}

/// The judged matches come best first with their scores; the judge saw each
/// match's location and its line with a line of context either side.
#[tokio::test]
async fn judged_matches_come_best_first_with_their_scores() {
    let (dir, matches) = workspace();
    let (ranking, judge) = ranking(Relevance::Scored(vec![Some(0.2), Some(0.9), Some(0.5)]), 30);
    let ranked = rank(
        Some(&ranking),
        "how retries are counted",
        matches,
        dir.path(),
        &Sandbox::new(None),
    )
    .await;
    assert_eq!(
        lines(&ranked.matches),
        [(4, Some(0.9)), (6, Some(0.5)), (2, Some(0.2))]
    );
    assert_eq!(ranked.notice, None);
    let seen = judge.seen.lock().unwrap();
    assert_eq!(seen[0].0, "how retries are counted");
    assert_eq!(
        seen[0].1[1],
        RelevanceCandidate {
            location: "src/lib.rs:4".into(),
            text: "b\nlet retry_count = 3;\nc".into(),
        }
    );
    match &ranked.record {
        RankingRecord::Ranked {
            candidates, scores, ..
        } => {
            assert_eq!(*candidates, 3);
            assert_eq!(scores[0].location, "src/lib.rs:4");
            assert_eq!(scores[0].score, Some(0.9));
        }
        other => panic!("{other:?}"),
    }
}

/// Only the first `max_candidates` are judged; the rest follow in rg order,
/// and unscored judged matches follow the scored ones.
#[tokio::test]
async fn beyond_the_candidate_cap_and_unscored_matches_follow_in_rg_order() {
    let (dir, matches) = workspace();
    let (ranking, judge) = ranking(Relevance::Scored(vec![None, Some(0.8)]), 2);
    let ranked = rank(
        Some(&ranking),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
    )
    .await;
    assert_eq!(judge.seen.lock().unwrap()[0].1.len(), 2);
    assert_eq!(
        lines(&ranked.matches),
        [(4, Some(0.8)), (2, None), (6, None)]
    );
    let notice = ranked.notice.unwrap();
    assert!(
        notice.contains("1 of 2 matches could not be judged"),
        "{notice}"
    );
    assert!(
        notice.contains("ranked the first 2 of 3 matches"),
        "{notice}"
    );
}

/// Ranking never fails a search: not configured, unavailable, or a wrong
/// number of scores keep rg's order and say why.
#[tokio::test]
async fn without_a_ranking_the_matches_keep_rg_order_and_say_why() {
    let (dir, matches) = workspace();
    let unranked = rank(None, "q", matches, dir.path(), &Sandbox::new(None)).await;
    assert_eq!(lines(&unranked.matches), [(2, None), (4, None), (6, None)]);
    assert_eq!(unranked.notice.as_deref(), Some(NOT_CONFIGURED));
    assert!(matches!(
        unranked.record,
        RankingRecord::NotConfigured { .. }
    ));

    let (dir, matches) = workspace();
    let (down, _) = ranking(
        Relevance::Unavailable("TypeSafe answered HTTP 529".into()),
        30,
    );
    let unranked = rank(Some(&down), "q", matches, dir.path(), &Sandbox::new(None)).await;
    assert_eq!(lines(&unranked.matches), [(2, None), (4, None), (6, None)]);
    assert_eq!(
        unranked.notice.as_deref(),
        Some("rank_by unavailable: TypeSafe answered HTTP 529; results are in rg order")
    );
    assert!(matches!(unranked.record, RankingRecord::Unavailable { .. }));

    let (dir, matches) = workspace();
    let (short, _) = ranking(Relevance::Scored(vec![Some(0.1)]), 30);
    let unranked = rank(Some(&short), "q", matches, dir.path(), &Sandbox::new(None)).await;
    assert_eq!(lines(&unranked.matches), [(2, None), (4, None), (6, None)]);
    assert!(
        unranked
            .notice
            .unwrap()
            .contains("the judge answered for 1 of 3 matches")
    );
}

/// What the judge sees keeps the match: each line is cut on its own, so a
/// long line before the match cannot push it out.
#[tokio::test]
async fn a_long_neighbouring_line_never_hides_the_match() {
    let dir = tempfile::TempDir::new().unwrap();
    let long = "x".repeat(5000);
    std::fs::write(dir.path().join("m.js"), format!("{long}\nretry_delay()\n")).unwrap();
    let matches = vec![RgMatch {
        file_path: dir.path().join("m.js"),
        line_number: 2,
        line_count: 1,
        score: None,
        column: Some(0),
    }];
    let (ranking, judge) = ranking(Relevance::Scored(vec![Some(0.5)]), 30);
    rank(
        Some(&ranking),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
    )
    .await;
    let seen = judge.seen.lock().unwrap();
    let text = &seen[0].1[0].text;
    assert!(text.ends_with("retry_delay()"), "{text:.60}");
    assert_eq!(text.chars().count(), 400 + 1 + "retry_delay()".len());
}

/// A match whose line cannot be read (past the file, or past the context
/// cache) is never sent to the judge and stays unscored after the judged
/// ones. (Paths also pass the sandbox's policy check before being read.)
#[tokio::test]
async fn an_unreadable_match_is_not_sent_and_stays_unscored() {
    let (dir, mut matches) = workspace();
    matches[1].line_number = 999; // past the end of the file
    let (ranking, judge) = ranking(Relevance::Scored(vec![Some(0.3), Some(0.8)]), 30);
    let ranked = rank(
        Some(&ranking),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
    )
    .await;
    let sent: Vec<String> = judge.seen.lock().unwrap()[0]
        .1
        .iter()
        .map(|c| c.location.clone())
        .collect();
    assert_eq!(sent, ["src/lib.rs:2", "src/lib.rs:6"]);
    assert_eq!(
        lines(&ranked.matches),
        [(6, Some(0.8)), (2, Some(0.3)), (999, None)]
    );
    assert!(
        ranked
            .notice
            .unwrap()
            .contains("1 of 3 matches could not be judged")
    );
}

/// PR #2138 review: a match past column 400 is still what the judge sees:
/// a long matched line is cut to a window around the match.
#[tokio::test]
async fn a_match_far_along_a_long_line_is_what_the_judge_sees() {
    let dir = tempfile::TempDir::new().unwrap();
    let filler = "x".repeat(450);
    std::fs::write(
        dir.path().join("m.js"),
        format!("{filler}retry_delay=5;{filler}\n"),
    )
    .unwrap();
    let matches = vec![RgMatch {
        file_path: dir.path().join("m.js"),
        line_number: 1,
        line_count: 1,
        score: None,
        column: Some(450),
    }];
    let (ranking, judge) = ranking(Relevance::Scored(vec![Some(0.5)]), 30);
    rank(
        Some(&ranking),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
    )
    .await;
    let text = judge.seen.lock().unwrap()[0].1[0].text.clone();
    assert!(text.contains("retry_delay=5"), "{text}");
    assert!(text.starts_with('…'), "the cut is marked: {text:.20}");
    assert!(text.chars().count() <= 402, "{}", text.chars().count());
}

/// rg's first submatch start is where the match sits in its line.
#[test]
fn the_match_column_comes_from_rgs_first_submatch() {
    let json = r#"{"type":"match","data":{"path":{"text":"/ws/a.rs"},"line_number":3,"lines":{"text":"let retry_delay = 5;\n"},"absolute_offset":0,"submatches":[{"match":{"text":"retry_delay"},"start":4,"end":15}]}}"#;
    let matches = super::super::parse_rg_matches(json);
    assert_eq!(matches[0].column, Some(4));
    assert_eq!(
        super::around(&"x".repeat(10), 4),
        "xxxxxxxxxx",
        "a short line is whole"
    );
}

/// When no judged match looks relevant, the agent is told so rather than
/// reading the best of poor matches as the answer.
#[tokio::test]
async fn no_relevant_match_is_said_so() {
    let (dir, matches) = workspace();
    let (poor, _) = ranking(Relevance::Scored(vec![Some(0.2), Some(0.12), None]), 30);
    let ranked = rank(Some(&poor), "q", matches, dir.path(), &Sandbox::new(None)).await;
    let notice = ranked.notice.unwrap();
    assert!(
        notice.contains("no judged match looks relevant (best 0.20)"),
        "{notice}"
    );
    let (dir, matches) = workspace();
    let (good, _) = ranking(Relevance::Scored(vec![Some(0.2), Some(0.8), Some(0.1)]), 30);
    let ranked = rank(Some(&good), "q", matches, dir.path(), &Sandbox::new(None)).await;
    assert_eq!(ranked.notice, None);
}
