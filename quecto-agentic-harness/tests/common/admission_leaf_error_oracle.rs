//! One executable acceptance oracle shared by actual-wire checks and observed
//! counterexamples. A mutation of each fact must fail its own named predicate.
use super::fixture::{Snapshot, Transcript};
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};

#[derive(Clone, Copy)]
pub struct Expected<'a> {
    pub post: bool,
    pub enabled: bool,
    pub throttle: bool,
    pub error: &'a str,
}
pub fn checks(
    sends: usize,
    output: &Transcript,
    state: &Snapshot,
    expected: Expected<'_>,
) -> Vec<(&'static str, bool)> {
    let Expected {
        post,
        enabled,
        throttle,
        error,
    } = expected;
    vec![
        ("send-once", sends == 1),
        (
            "text-once",
            output.text == if post { vec!["visible once"] } else { vec![] },
        ),
        (
            "exact-error",
            output.errors == if enabled { vec![error] } else { vec![] },
        ),
        ("terminal-kind", output.done == usize::from(!enabled)),
        ("grant-once", state.grants == usize::from(enabled)),
        (
            "receipt-count",
            state.receipts.len() == usize::from(enabled && throttle),
        ),
        (
            "receipt-kind",
            !(enabled && throttle)
                || matches!(
                    state.receipts.first(),
                    Some(ThrottleFeedback::NoHint { .. })
                ),
        ),
        (
            "failure-finish-once",
            state.finishes
                == if enabled {
                    vec![Feedback::Failure]
                } else {
                    vec![]
                },
        ),
        ("no-abandonment", state.abandoned == 0),
    ]
}
pub fn verify(checks: Vec<(&'static str, bool)>) {
    let failed: Vec<_> = checks
        .into_iter()
        .filter_map(|(name, ok)| (!ok).then_some(name))
        .collect();
    assert!(
        failed.is_empty(),
        "leaf error acceptance failures: {failed:?}"
    );
}
#[test]
fn every_leaf_error_predicate_rejects_its_observed_counterexample() {
    for (post, enabled, throttle) in [
        (false, true, true),
        (true, true, true),
        (true, true, false),
        (true, false, false),
    ] {
        let output = || Transcript {
            text: if post {
                vec!["visible once".into()]
            } else {
                vec![]
            },
            errors: if enabled {
                vec!["original".into()]
            } else {
                vec![]
            },
            done: usize::from(!enabled),
        };
        let state = || {
            let mut state = Snapshot::default();
            state.grants = usize::from(enabled);
            if enabled {
                state.finishes = vec![Feedback::Failure];
            }
            if enabled && throttle {
                state.receipts = vec![ThrottleFeedback::NoHint { jitter: 0 }];
            }
            state
        };
        verify(checks(
            1,
            &output(),
            &state(),
            Expected {
                post,
                enabled,
                throttle,
                error: "original",
            },
        ));
        for name in [
            "send-once",
            "text-once",
            "exact-error",
            "terminal-kind",
            "grant-once",
            "receipt-count",
            "receipt-kind",
            "failure-finish-once",
            "no-abandonment",
        ] {
            if name == "receipt-kind" && !(enabled && throttle) {
                continue;
            }
            let mut out = output();
            let mut s = state();
            let mut sends = 1;
            match name {
                "send-once" => sends = 2,
                "text-once" => out.text.push("replayed".into()),
                "exact-error" => out.errors.push("changed".into()),
                "terminal-kind" => out.done += 1,
                "grant-once" => s.grants += 1,
                "receipt-count" => s.receipts.push(ThrottleFeedback::Unavailable),
                "receipt-kind" => s.receipts[0] = ThrottleFeedback::Unavailable,
                "failure-finish-once" => s.finishes.push(Feedback::Success),
                "no-abandonment" => s.abandoned = 1,
                _ => unreachable!(),
            }
            assert_eq!(
                checks(
                    sends,
                    &out,
                    &s,
                    Expected {
                        post,
                        enabled,
                        throttle,
                        error: "original"
                    }
                )
                .into_iter()
                .find(|(id, _)| *id == name),
                Some((name, false)),
                "{name} accepted its counterexample"
            );
        }
    }
}
