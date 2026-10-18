use super::append_feedback;
use crate::domain::conversation::services::turn_origin::{TurnOrigin, instruction, progress_nudge};
use crate::domain::conversation::value_objects::message::Message;

/// #2226: the loop's own feedback is not an instruction: a pushed feedback
/// message takes the origin of the turn already open when it is added.
#[test]
fn pushed_feedback_takes_the_origin_of_the_open_turn() {
    for (opener, origin) in [
        (progress_nudge("continue".into()), TurnOrigin::ProgressNudge),
        (instruction("task".into()), TurnOrigin::Instruction),
    ] {
        let mut messages = vec![opener, Message::assistant("", vec![])];
        append_feedback(&mut messages, "try again".into(), 3);
        assert_eq!(messages.len(), 3, "added as its own message");
        let feedback = messages.last().unwrap();
        assert_eq!(feedback.turn_origin, origin);
        assert_eq!(feedback.turn, Some(3));
    }
    let mut unstamped = vec![Message::assistant("", vec![])];
    append_feedback(&mut unstamped, "try again".into(), 1);
    assert_eq!(unstamped[1].turn_origin, TurnOrigin::Unknown);
}

/// #2403/#2414: feedback after a trailing user message is its own message
/// too: the message already sent keeps its text and its origin.
#[test]
fn feedback_after_a_user_message_never_edits_it() {
    let mut messages = vec![progress_nudge("continue".into())];
    append_feedback(&mut messages, "try again".into(), 1);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "continue");
    assert_eq!(messages[0].turn_origin, TurnOrigin::ProgressNudge);
    assert_eq!(messages[1].turn_origin, TurnOrigin::ProgressNudge);
}
