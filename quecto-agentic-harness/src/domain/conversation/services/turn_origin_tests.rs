use super::{
    TurnOrigin, current_phase, harness_note, instruction, is_substantive_reply, progress_nudge,
    report_index, stamp_turn, transcript_report_index,
};
use crate::domain::conversation::value_objects::message::{Message, Role, ToolCall};

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
    assert!(I.is_stamped() && N.is_stamped() && X.is_stamped() && !U.is_stamped());
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

/// A spilled reply of `origin` in loop turn `turn`.
fn spilled(text: &str, origin: TurnOrigin, turn: u32) -> Message {
    let mut message = stamped(reply(text), origin);
    message.turn = Some(turn);
    message.spill_id = Some(format!("turn{turn}:msg:assistant"));
    message
}

/// #2246 review finding 2: a harness note between the task and the answer
/// opens the answer's turn; with the nudge phase after it, the answer is
/// the report to keep.
#[test]
fn a_note_before_the_answer_never_hides_the_report_to_keep() {
    let task = instruction("task".into());
    let note = harness_note(
        "<subagent_notification/>".into(),
        std::slice::from_ref(&task),
    );
    assert_eq!(note.turn_origin, I, "the note lands in the task's phase");
    let messages = [
        task,
        note,
        spilled("ANSWER", I, 1),
        progress_nudge("continue".into()),
        spilled("status", N, 1),
        progress_nudge("continue".into()),
    ];
    assert_eq!(super::report_to_keep(&messages), Some(2));
}

/// #2246 review finding 2: when no turn is in flight (a resumed transcript
/// pruned before its next prompt), the latest opener's turn is finished,
/// so the answer it holds is kept, even when a note opened that turn.
#[test]
fn a_finished_latest_turn_keeps_its_report() {
    let task = instruction("task".into());
    let note = harness_note(
        "<subagent_notification/>".into(),
        std::slice::from_ref(&task),
    );
    let messages = [task, note, spilled("ANSWER", I, 1)];
    assert_eq!(super::report_to_keep(&messages), Some(2));
    let messages = [instruction("task".into()), spilled("ANSWER", I, 1)];
    assert_eq!(super::report_to_keep(&messages), Some(1));
}

/// The turn in flight (its replies not yet stamped) never gives the report
/// to keep, even when its reply would outrank every finished one.
#[test]
fn a_turn_in_flight_never_gives_the_report_to_keep() {
    let messages = [
        instruction("task".into()),
        spilled("status", N, 1),
        progress_nudge("continue".into()),
        spilled("in flight", U, 1),
    ];
    assert_eq!(super::report_to_keep(&messages), Some(1));
    // An unmarked opener (a note in a session saved before #2226) cannot say
    // whether its turn finished: it is taken as in flight.
    let messages = [Message::user("task"), spilled("LEGACY", U, 1)];
    assert_eq!(super::report_to_keep(&messages), None);
}

/// The loop's own feedback inside the turn in flight is stamped when it is
/// added, but it opens no turn and does not finish the turn: every reply of
/// that turn stays out of the report to keep.
#[test]
fn stamped_feedback_never_finishes_the_turn_in_flight() {
    let mut feedback = stamped(Message::user("Your reply was cut off."), N);
    feedback.turn = Some(1);
    let messages = [
        instruction("task".into()),
        spilled("status", N, 1),
        progress_nudge("continue".into()),
        spilled("cut off", U, 1),
        feedback,
        spilled("in flight", U, 2),
    ];
    assert_eq!(super::report_to_keep(&messages), Some(1));
}

/// Every opener pruned away: no turn is known to be in flight, so the
/// report among what is left is kept.
#[test]
fn a_transcript_without_an_opener_keeps_its_report() {
    let messages = [spilled("ANSWER", I, 1), spilled("status", N, 2)];
    assert_eq!(super::report_to_keep(&messages), Some(0));
}

/// An unmarked opener cannot say whether its turn finished, even when a
/// newer build stamped what followed it: the turn is taken as in flight.
#[test]
fn an_unmarked_opener_is_taken_as_in_flight() {
    use TurnOrigin::Unrecognised as X;
    let messages = [Message::user("note"), spilled("reply", X, 1)];
    assert_eq!(super::report_to_keep(&messages), None);
}

/// #2246 cold review N3: one rule finds the latest opener, for the ceiling's
/// region and for the report to keep: a user message the loop did not
/// append inside a turn.
#[test]
fn the_latest_opener_is_the_latest_user_message_outside_a_turn() {
    let mut feedback = Message::user("Your reply was cut off.");
    feedback.turn = Some(1);
    let messages = [
        instruction("task".into()),
        reply("status"),
        progress_nudge("continue".into()),
        feedback,
        reply("done"),
    ];
    assert_eq!(super::latest_opener(&messages), Some(2));
    assert_eq!(super::latest_opener(&messages[..2]), Some(0));
    assert_eq!(super::latest_opener(&messages[1..2]), None);
    assert_eq!(super::latest_opener(&[]), None);
}

/// #2403 review L5: a watermark cut's stub opens no turn, whatever mode
/// the session runs in later.
#[test]
fn an_archive_stub_opens_no_turn() {
    use crate::domain::conversation::services::watermark_cut::archive_stub;
    assert!(!super::opens_turn(&archive_stub(3, Some("archive"))));
    assert!(super::opens_turn(
        &crate::domain::conversation::services::turn_origin::prompt("p".into())
    ));
    assert!(super::opens_turn(&harness_note("a wake".into(), &[])));
    let messages = vec![
        crate::domain::conversation::services::turn_origin::prompt("p".into()),
        archive_stub(3, Some("archive")),
    ];
    assert_eq!(super::latest_opener(&messages), Some(0));
}
