//! Exact shared promise-settlement assertions for the UDS parent tests.
use super::{CORRECTION_NOTE, STALLED_REASON, Value, of_type, position};

fn assert_stalled_settlement(requests: &[String], frames: &[Value]) {
    assert_eq!(
        requests.len(),
        2,
        "exactly two provider requests: {requests:?}"
    );
    assert_eq!(requests[1], CORRECTION_NOTE, "the literal correction, once");
    assert_eq!(
        requests
            .iter()
            .filter(|note| note.as_str() == CORRECTION_NOTE)
            .count(),
        1
    );
    let admitted = of_type(frames, "turn_admitted");
    assert_eq!(
        admitted.len(),
        2,
        "the opener and the correction: {frames:?}"
    );
    assert!(
        admitted
            .iter()
            .all(|frame| frame["episodeId"] == "ep-1" && frame.as_object().unwrap().len() == 5)
    );
    assert_ne!(admitted[0]["turnId"], admitted[1]["turnId"]);
    let outcomes = of_type(frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "one outcome, never done: {frames:?}");
    assert_eq!(outcomes[0]["outcome"], "stalled");
    assert_eq!(outcomes[0]["reason"], STALLED_REASON);
    assert_eq!(outcomes[0]["turnId"], admitted[1]["turnId"]);
    assert_eq!(outcomes[0]["sequence"], 1);
    assert_eq!(outcomes[0].as_object().unwrap().len(), 8);
}

/// Ordinary default clients receive actual execution success only at END.
pub(super) fn assert_stalled(requests: &[String], frames: &[Value]) {
    assert_stalled_settlement(requests, frames);
    let response = position(frames, |f| f["type"] == "response" && f["id"] == "ep-1");
    let first = position(frames, |f| f["type"] == "turn_admitted");
    let start = position(frames, |f| f["type"] == "agent_start");
    let outcome = position(frames, |f| f["type"] == "turn_outcome");
    let last_end = frames
        .iter()
        .rposition(|f| f["type"] == "agent_end")
        .unwrap();
    assert!(
        first < start && last_end < outcome && outcome < response,
        "{frames:?}"
    );
    assert_eq!(frames[response]["command"], "prompt");
    assert_eq!(
        frames[response]["success"], true,
        "stalled is not an execution error"
    );
    assert_eq!(
        of_type(frames, "response")
            .iter()
            .filter(|f| f["id"] == "ep-1")
            .count(),
        1,
        "exactly one END response"
    );
}

/// Production env1 alone retains the original admission receipt contract.
pub(super) fn assert_stalled_opt_in(requests: &[String], frames: &[Value]) {
    assert_stalled_settlement(requests, frames);
    // Order: ack, admitted, start, end, admitted, start, end, outcome.
    let ack = position(frames, |f| f["type"] == "response" && f["id"] == "ep-1");
    let first = position(frames, |f| f["type"] == "turn_admitted");
    let start = position(frames, |f| f["type"] == "agent_start");
    let outcome = position(frames, |f| f["type"] == "turn_outcome");
    let last_end = frames
        .iter()
        .rposition(|f| f["type"] == "agent_end")
        .unwrap();
    assert!(
        ack < first && first < start && last_end < outcome,
        "{frames:?}"
    );
    assert_eq!(frames[ack]["data"]["episodeId"], "ep-1");
    assert_eq!(frames[ack]["command"], "prompt");
    assert_eq!(frames[ack]["success"], true);
    assert_eq!(
        of_type(frames, "response")
            .iter()
            .filter(|f| f["id"] == "ep-1")
            .count(),
        1,
        "one acknowledgement, at admission"
    );
}

/// Whether any file under `dir` holds `needle`.
pub(super) fn saved_under(dir: &std::path::Path, needle: &str) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            match path.is_dir() {
                true => saved_under(&path, needle),
                false => std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle)),
            }
        })
}
