use super::*;

fn delta(next_rev: Option<u64>) -> TranscriptDelta {
    TranscriptDelta {
        epoch: 3,
        rev: 9,
        messages: vec![Message::user("committed")],
        next_rev,
    }
}

#[test]
fn a_delta_is_caught_up_exactly_when_it_was_not_cut() {
    assert!(delta(None).caught_up());
    let cut = delta(Some(7));
    assert!(!cut.caught_up());
    assert_eq!(cut.next_rev, Some(7));
}

#[test]
fn the_request_and_replies_are_plain_values() {
    let request = SyncRequest {
        epoch: 1,
        since_rev: 2,
        reset_window: 64,
    };
    assert_eq!(request.clone(), request);
    let reset = TranscriptSync::Reset(TranscriptReset {
        epoch: 4,
        rev: 2,
        page: HistoryPage {
            messages: Vec::new(),
            before: None,
            has_more_before: false,
        },
    });
    assert!(format!("{reset:?}").starts_with("Reset"));
    assert!(format!("{:?}", TranscriptSync::Delta(delta(None))).starts_with("Delta"));
}
