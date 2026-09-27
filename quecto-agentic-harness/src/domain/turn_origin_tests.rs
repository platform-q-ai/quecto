use super::{
    TurnOrigin, current_phase, harness_note, instruction, is_substantive_reply, progress_nudge,
    report_index, stamp_turn, transcript_report_index,
};
use crate::domain::message::{Message, Role, ToolCall};

use TurnOrigin::{Instruction as I, ProgressNudge as N, Unknown as U};

fn stamped(mut message: Message, origin: TurnOrigin) -> Message {
    message.turn_origin = origin;
    message
}

fn reply(text: &str) -> Message {
    Message::assistant(text, vec![])
}

fn call() -> ToolCall {
    ToolCall {
        id: "call-1".into(),
        name: "workflow".into(),
        arguments: "{}".into(),
    }
}

#[test]
fn openers_are_stamped_with_their_origin() {
    let task = instruction("task".into());
    assert_eq!((task.role.clone(), task.turn_origin), (Role::User, I));
    let nudge = progress_nudge("continue".into());
    assert_eq!((nudge.role.clone(), nudge.turn_origin), (Role::User, N));
    assert_eq!(nudge.content, "continue");
    assert_eq!(Message::user("plain").turn_origin, U);
}

#[test]
fn ranks_answers_and_stamps() {
    use TurnOrigin::Unrecognised as X;
    assert!(I.report_rank() > U.report_rank());
    assert!(U.report_rank() > N.report_rank());
    assert!(N.report_rank() > X.report_rank());
    assert!(I.answers() && U.answers() && !N.answers() && !X.answers());
    assert!(I.is_known() && N.is_known() && !U.is_known() && !X.is_known());
    assert_eq!(TurnOrigin::default(), U);
}

#[test]
fn a_harness_note_opens_a_turn_of_the_phase_it_lands_in() {
    let answered = [
        stamped(Message::user("task"), I),
        stamped(reply("REPORT"), I),
    ];
    assert_eq!(harness_note("note".into(), &answered).turn_origin, I);
    let nudged = [
        stamped(reply("REPORT"), I),
        stamped(Message::user("continue"), N),
        stamped(reply("status"), N),
        Message::assistant("unstamped", vec![]),
    ];
    assert_eq!(current_phase(&nudged), N);
    assert_eq!(harness_note("note".into(), &nudged).turn_origin, N);
    assert_eq!(current_phase(&[]), U);
    assert_eq!(current_phase(&[reply("legacy")]), U);
}

#[test]
fn stamping_a_turn_gives_every_later_unstamped_message_the_openers_origin() {
    let nudge = progress_nudge("continue".into());
    let id = nudge.id();
    let mut messages = vec![
        stamped(Message::user("task"), I),
        stamped(reply("REPORT"), I),
        nudge,
        Message::assistant("", vec![call()]),
        Message::tool("call-1", "ok"),
        // The loop's own feedback inside the turn is not an instruction.
        Message::user("Your reply was cut off; answer concisely."),
        reply("status"),
    ];
    assert_eq!(stamp_turn(&mut messages, id), 4);
    let origins: Vec<TurnOrigin> = messages.iter().map(|m| m.turn_origin).collect();
    assert_eq!(origins, [I, I, N, N, N, N, N]);
    // Stamps are never overwritten.
    assert_eq!(stamp_turn(&mut messages, id), 0);
}

#[test]
fn a_turn_whose_opener_is_gone_stays_unknown() {
    let mut messages = vec![stamped(Message::user("task"), I), reply("status")];
    let gone = progress_nudge("continue".into()).id();
    assert_eq!(stamp_turn(&mut messages, gone), 0);
    assert_eq!(messages[1].turn_origin, U);
}

#[test]
fn a_substantive_reply_is_non_blank_assistant_text_without_tool_calls() {
    assert!(is_substantive_reply(&reply("REPORT")));
    assert!(!is_substantive_reply(&reply(" \n")));
    assert!(!is_substantive_reply(&Message::user("task")));
    assert!(!is_substantive_reply(&Message::assistant(
        "calling",
        vec![call()]
    )));
}

/// (substantive, origin)
type Candidate = (bool, TurnOrigin);

fn report_of(candidates: &[Candidate]) -> Option<usize> {
    report_index(candidates.len(), |i| candidates[i].0, |i| candidates[i].1)
}

#[test]
fn the_report_is_the_latest_substantive_answer_to_an_instruction() {
    // A report, then nudge replies and unmarked ones: the report stands.
    assert_eq!(
        report_of(&[(true, I), (false, I), (true, N), (true, U)]),
        Some(0)
    );
    // A later instruction's answer replaces an earlier one.
    assert_eq!(report_of(&[(true, I), (true, N), (true, I)]), Some(2));
    assert_eq!(report_of(&[(true, I), (false, I)]), Some(0));
}

#[test]
fn without_an_answer_an_unmarked_reply_outranks_a_nudge_reply() {
    // Review probe P4: a session saved before #2226 answered unmarked; the
    // nudge replies after it, stamped by an upgraded build, never replace it.
    assert_eq!(report_of(&[(false, U), (true, U), (true, N)]), Some(1));
    assert_eq!(report_of(&[(true, N), (true, U), (true, N)]), Some(1));
    // Only nudge replies: the latest of them, so a report is never withheld.
    assert_eq!(
        report_of(&[(false, I), (true, N), (true, N), (false, N)]),
        Some(2)
    );
    assert_eq!(report_of(&[(false, I), (false, N)]), None);
    assert_eq!(report_of(&[]), None);
    // Review 3 L2: an origin this build does not know ranks below a nudge.
    let x = TurnOrigin::Unrecognised;
    assert_eq!(report_of(&[(true, x), (true, N)]), Some(1));
    assert_eq!(report_of(&[(true, N), (true, x)]), Some(0));
    assert_eq!(report_of(&[(true, x)]), Some(0));
}

/// Review probe P4, end to end over a transcript.
#[test]
fn a_legacy_answer_then_upgraded_nudges_reports_the_legacy_answer() {
    let messages = [
        Message::user("task"),
        reply("LEGACY ANSWER"),
        progress_nudge("continue".into()),
        stamped(reply("nudge reply"), N),
    ];
    let report = transcript_report_index(&messages, is_substantive_reply).unwrap();
    assert_eq!(messages[report].content, "LEGACY ANSWER");
}

#[test]
fn a_report_ref_names_the_message_without_its_text() {
    let mut message = stamped(reply("REPORT"), I);
    message.ordinal = Some(7);
    let named = super::ReportRef::of(&message);
    assert_eq!(named.id, message.id().to_string());
    assert_eq!(
        (named.ordinal, named.origin, named.content_length),
        (Some(7), I, 6)
    );
}

#[test]
fn a_transcript_reports_by_each_messages_own_stamp() {
    let messages = [
        stamped(Message::user("task"), I),
        stamped(reply("REPORT"), I),
        stamped(Message::user("continue"), N),
        stamped(reply("status"), N),
    ];
    assert_eq!(
        transcript_report_index(&messages, is_substantive_reply),
        Some(1)
    );
    // The answer's opener pruned away: its stamp still holds.
    assert_eq!(
        transcript_report_index(&messages[1..], is_substantive_reply),
        Some(0)
    );
    assert_eq!(
        transcript_report_index(&messages[2..], is_substantive_reply),
        Some(1)
    );
    assert_eq!(transcript_report_index(&[], is_substantive_reply), None);
}
