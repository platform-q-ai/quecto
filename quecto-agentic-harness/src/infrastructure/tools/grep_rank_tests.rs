use super::*;
use crate::application::search::ports::PortFuture;
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
    let ranked = rank(Some(&ranking), "q", matches, dir.path()).await;
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
    let unranked = rank(None, "q", matches, dir.path()).await;
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
    let unranked = rank(Some(&down), "q", matches, dir.path()).await;
    assert_eq!(lines(&unranked.matches), [(2, None), (4, None), (6, None)]);
    assert_eq!(
        unranked.notice.as_deref(),
        Some("rank_by unavailable: TypeSafe answered HTTP 529; results are in rg order")
    );
    assert!(matches!(unranked.record, RankingRecord::Unavailable { .. }));

    let (dir, matches) = workspace();
    let (short, _) = ranking(Relevance::Scored(vec![Some(0.1)]), 30);
    let unranked = rank(Some(&short), "q", matches, dir.path()).await;
    assert_eq!(lines(&unranked.matches), [(2, None), (4, None), (6, None)]);
    assert!(
        unranked
            .notice
            .unwrap()
            .contains("the judge answered for 1 of 3 matches")
    );
}
