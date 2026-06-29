use super::*;

fn note(agent: &str) -> PendingMessage {
    PendingMessage::subagent_notification(agent.into(), 1, format!("{agent} done"), true)
}

fn failure_note(agent: &str, content: &str) -> PendingMessage {
    PendingMessage::subagent_notification(agent.into(), 1, content.into(), false)
}

/// A burst of N completions buffered during one busy parent turn collapses to
/// exactly ONE informational summary note at the idle flush (#894 AC#1/#5).
#[test]
fn burst_of_completions_coalesces_to_single_note() {
    let pending: Vec<PendingMessage> = (0..5).map(|i| note(&format!("basic-{i}"))).collect();
    let out = coalesce_pending(pending);
    assert_eq!(out.len(), 1, "K>1 notes must collapse to one, got {out:?}");
    let content = out.into_iter().next().unwrap().into_message().content;
    assert!(content.contains("5 sub-agents finished"), "got: {content}");
    assert!(content.contains("basic-0"), "names listed, got: {content}");
    assert!(content.contains("basic-4"), "names listed, got: {content}");
    // Informational, not imperative (#894 AC#2).
    assert!(content.contains("get_messages"), "got: {content}");
    assert!(
        !content.contains("ready for inspection"),
        "must not read as a standing order, got: {content}"
    );
}

/// Past the name cap the tail is summarized as "(+M more)" (#894 AC#1).
#[test]
fn coalesced_note_caps_name_list() {
    let pending: Vec<PendingMessage> = (0..13).map(|i| note(&format!("w{i}"))).collect();
    let out = coalesce_pending(pending);
    assert_eq!(out.len(), 1);
    let content = out.into_iter().next().unwrap().into_message().content;
    assert!(content.contains("13 sub-agents finished"), "got: {content}");
    assert!(content.contains("(+3 more)"), "cap tail, got: {content}");
}

/// A single completion passes through unchanged — one clean one-line note
/// (#894 AC#3).
#[test]
fn single_completion_is_not_coalesced() {
    let out = coalesce_pending(vec![note("solo")]);
    assert_eq!(out.len(), 1);
    match &out[0] {
        PendingMessage::SubagentNotification { agent_id, .. } => {
            assert_eq!(agent_id, "solo");
        }
        other => panic!("single note must pass through untouched, got {other:?}"),
    }
}

/// At exactly the cap, all names are listed and there is NO "(+M more)" tail;
/// one past the cap appends "(+1 more)" — guards the `total > shown` off-by-one
/// (#894 AC#1).
#[test]
fn coalesced_note_cap_boundary() {
    let at_cap: Vec<PendingMessage> = (0..COALESCE_NAME_CAP)
        .map(|i| note(&format!("w{i}")))
        .collect();
    let content = coalesce_pending(at_cap)
        .into_iter()
        .next()
        .unwrap()
        .into_message()
        .content;
    assert!(content.contains("10 sub-agents finished"), "got: {content}");
    assert!(!content.contains("more)"), "no tail at cap, got: {content}");

    let past_cap: Vec<PendingMessage> = (0..COALESCE_NAME_CAP + 1)
        .map(|i| note(&format!("w{i}")))
        .collect();
    let content = coalesce_pending(past_cap)
        .into_iter()
        .next()
        .unwrap()
        .into_message()
        .content;
    assert!(content.contains("11 sub-agents finished"), "got: {content}");
    assert!(
        content.contains("(+1 more)"),
        "tail at cap+1, got: {content}"
    );
}

/// A mixed batch (completions + a failure) must NOT launder the failure into
/// "finished": only completions coalesce, and the errored note passes through
/// verbatim so its error detail survives (#894 Finding 1).
#[test]
fn errored_notes_are_not_coalesced_into_finished() {
    let pending = vec![
        note("ok-1"),
        failure_note("bad", "Agent 'bad' failed: boom"),
        note("ok-2"),
    ];
    let out = coalesce_pending(pending);
    assert_eq!(
        out.len(),
        2,
        "one coalesced completion note + one failure note, got {out:?}"
    );
    // The coalesced summary counts ONLY the two completions, not the failure.
    let coalesced = out
        .iter()
        .find_map(|m| match m {
            PendingMessage::CoalescedSubagentNotification { content } => Some(content.clone()),
            _ => None,
        })
        .expect("a coalesced completion note");
    assert!(
        coalesced.contains("2 sub-agents finished"),
        "failure must not inflate the finished count, got: {coalesced}"
    );
    assert!(
        !coalesced.contains("bad"),
        "failure not in summary: {coalesced}"
    );
    // The failure note survives verbatim with its error detail.
    let failure = out
        .iter()
        .find_map(|m| match m {
            PendingMessage::SubagentNotification {
                is_completion: false,
                content,
                ..
            } => Some(content.clone()),
            _ => None,
        })
        .expect("the errored note passes through individually");
    assert!(
        failure.contains("failed: boom"),
        "error detail survives: {failure}"
    );
}

/// A lone completion alongside a failure: neither coalesces (only one
/// completion), both pass through with their detail intact (#894 Finding 1).
#[test]
fn single_completion_plus_failure_both_pass_through() {
    let pending = vec![
        note("ok"),
        failure_note("bad", "Agent 'bad' exited unexpectedly"),
    ];
    let out = coalesce_pending(pending);
    assert_eq!(
        out.len(),
        2,
        "no coalescing with a single completion, got {out:?}"
    );
    assert!(
        out.iter()
            .all(|m| matches!(m, PendingMessage::SubagentNotification { .. }))
    );
}

/// Non-notification pending messages (steer/follow-up) are preserved; only the
/// notifications collapse.
#[test]
fn user_messages_are_preserved() {
    let pending = vec![
        PendingMessage::user("steer me".into()),
        note("a"),
        note("b"),
    ];
    let out = coalesce_pending(pending);
    assert_eq!(
        out.len(),
        2,
        "one user msg + one coalesced note, got {out:?}"
    );
    assert!(matches!(out[0], PendingMessage::User(_)));
    let content = out[1].clone().into_message().content;
    assert!(content.contains("2 sub-agents finished"), "got: {content}");
}
