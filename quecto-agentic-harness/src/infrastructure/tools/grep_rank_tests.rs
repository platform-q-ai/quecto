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
/// match's location and its line with three lines of context either side
/// (here the whole file).
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
        true,
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
            before: "a\nfn retry() {}\nb".into(),
            matched: "let retry_count = 3;".into(),
            after: "c\n// retry later\nd".into(),
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
        true,
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
    let unranked = rank(None, "q", matches, dir.path(), &Sandbox::new(None), true).await;
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
    let unranked = rank(
        Some(&down),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    assert_eq!(lines(&unranked.matches), [(2, None), (4, None), (6, None)]);
    assert_eq!(
        unranked.notice.as_deref(),
        Some("rank_by unavailable: TypeSafe answered HTTP 529; results are in rg order")
    );
    assert!(matches!(unranked.record, RankingRecord::Unavailable { .. }));

    let (dir, matches) = workspace();
    let (short, _) = ranking(Relevance::Scored(vec![Some(0.1)]), 30);
    let unranked = rank(
        Some(&short),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
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
        true,
    )
    .await;
    let seen = judge.seen.lock().unwrap();
    let seen = &seen[0].1[0];
    assert_eq!(seen.matched, "retry_delay()");
    assert_eq!(seen.before.chars().count(), 400);
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
        true,
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
        true,
    )
    .await;
    let text = judge.seen.lock().unwrap()[0].1[0].matched.clone();
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
    let (poor, _) = ranking(
        Relevance::Scored(vec![Some(0.2), Some(0.12), Some(0.1)]),
        30,
    );
    let ranked = rank(
        Some(&poor),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    let notice = ranked.notice.unwrap();
    assert!(
        notice.contains("no judged match looks relevant (best 0.20)"),
        "{notice}"
    );
    let (dir, matches) = workspace();
    let (good, _) = ranking(Relevance::Scored(vec![Some(0.2), Some(0.8), Some(0.1)]), 30);
    let ranked = rank(
        Some(&good),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    assert_eq!(ranked.notice, None);
}

/// With some matches unjudged, the relevant one may be among them: no
/// "none looks relevant" claim, only the unjudged note.
#[tokio::test]
async fn no_relevance_claim_while_some_matches_are_unjudged() {
    let (dir, matches) = workspace();
    let (partly, _) = ranking(
        Relevance::Partial(
            vec![Some(0.2), None, None],
            "TypeSafe did not answer within 30 s".into(),
        ),
        30,
    );
    let ranked = rank(
        Some(&partly),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    let notice = ranked.notice.unwrap();
    assert!(!notice.contains("looks relevant"), "{notice}");
    assert!(
        notice.contains("2 of 3 matches could not be judged (TypeSafe did not answer within 30 s)"),
        "{notice}"
    );
}

/// Unread matches (rg's read cap) or matches past the candidate cap may hold
/// the relevant one: no "none looks relevant" claim for either.
#[tokio::test]
async fn no_relevance_claim_while_some_matches_are_unread_or_uncapped() {
    let (dir, matches) = workspace();
    let (poor, _) = ranking(
        Relevance::Scored(vec![Some(0.2), Some(0.12), Some(0.1)]),
        30,
    );
    let unread = rank(
        Some(&poor),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        false,
    )
    .await;
    assert_eq!(unread.notice, None);
    let (dir, matches) = workspace();
    let (capped, _) = ranking(Relevance::Scored(vec![Some(0.2), Some(0.1)]), 2);
    let ranked = rank(
        Some(&capped),
        "q",
        matches,
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    let notice = ranked.notice.unwrap();
    assert!(!notice.contains("looks relevant"), "{notice}");
    assert!(notice.contains("ranked the first 2 of 3"), "{notice}");
}

/// #2144: a doc comment matched two lines above the function it documents
/// shows the judge that function's signature; the context stops at three
/// lines either side and at the file's ends.
#[tokio::test]
async fn the_judge_sees_the_function_a_matched_comment_documents() {
    let dir = tempfile::TempDir::new().unwrap();
    let source = [
        "use std::time::Duration;",
        "",
        "impl Policy {",
        "    /// The delay to wait before the next attempt. Honours a `Retry-After`",
        "    /// hint when present; otherwise uses bounded exponential backoff.",
        "    fn backoff_delay(&self, attempt: u32) -> Duration {",
        "        self.base * 2u32.pow(attempt)",
        "    }",
        "}",
    ];
    std::fs::write(dir.path().join("retry.rs"), source.join("\n") + "\n").unwrap();
    let at = |line_number: usize| RgMatch {
        file_path: dir.path().join("retry.rs"),
        line_number,
        line_count: 1,
        score: None,
        column: None,
    };
    let (judged, judge) = ranking(Relevance::Scored(vec![Some(0.9), Some(0.1)]), 30);
    rank(
        Some(&judged),
        "where the retry delay is computed",
        vec![at(4), at(1)],
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    let seen = judge.seen.lock().unwrap();
    // Lines 1..=7 around line 4: the signature two lines below is in view
    // as context, line 8 is not.
    assert_eq!(seen[0].1[0].before, source[0..3].join("\n"));
    assert_eq!(seen[0].1[0].matched, source[3]);
    assert_eq!(seen[0].1[0].after, source[4..7].join("\n"));
    // Line 1 has nothing above it.
    assert_eq!(seen[0].1[1].before, "");
    assert_eq!(seen[0].1[1].after, source[1..4].join("\n"));
}

/// Context stops at the file's end; a match spanning more lines than a
/// judge is shown has its first ten judged and the rest as context.
#[tokio::test]
async fn context_stops_at_the_end_and_long_matches_are_clamped() {
    let dir = tempfile::TempDir::new().unwrap();
    let source: Vec<String> = (1..=16).map(|n| format!("line {n}")).collect();
    std::fs::write(dir.path().join("f.rs"), source.join("\n") + "\n").unwrap();
    let span = |line_number: usize, line_count: usize| RgMatch {
        file_path: dir.path().join("f.rs"),
        line_number,
        line_count,
        score: None,
        column: None,
    };
    let (judged, judge) = ranking(Relevance::Scored(vec![None, None]), 30);
    rank(
        Some(&judged),
        "q",
        vec![span(15, 1), span(2, 12)],
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    let seen = judge.seen.lock().unwrap();
    // Line 15 of 16: one line after it.
    assert_eq!(seen[0].1[0].before, source[11..14].join("\n"));
    assert_eq!(seen[0].1[0].after, source[15]);
    // Lines 2..=13 matched: 2..=11 judged, 12..=14 context.
    assert_eq!(seen[0].1[1].before, source[0]);
    assert_eq!(seen[0].1[1].matched, source[1..11].join("\n"));
    assert_eq!(seen[0].1[1].after, source[11..14].join("\n"));
}

/// A file past the 1 MiB context cache: the last line read may be cut, so
/// it is never shown, as context or as a match.
#[tokio::test]
async fn a_line_cut_by_the_file_cache_is_never_shown() {
    let dir = tempfile::TempDir::new().unwrap();
    let long = "x".repeat(2 * 1024 * 1024);
    std::fs::write(dir.path().join("big.rs"), format!("retry\nnext\n{long}\n")).unwrap();
    let at = |line_number: usize| RgMatch {
        file_path: dir.path().join("big.rs"),
        line_number,
        line_count: 1,
        score: None,
        column: None,
    };
    let (judged, judge) = ranking(Relevance::Scored(vec![Some(0.5)]), 30);
    rank(
        Some(&judged),
        "q",
        vec![at(1), at(3)],
        dir.path(),
        &Sandbox::new(None),
        true,
    )
    .await;
    let seen = judge.seen.lock().unwrap();
    // Only the first match is sent, and without the cut third line.
    assert_eq!(seen[0].1.len(), 1);
    assert_eq!(seen[0].1[0].matched, "retry");
    assert_eq!(seen[0].1[0].after, "next");
}
