use super::{Feedback, append_feedback};
use crate::domain::message::Message;
use crate::domain::turn_origin::{TurnOrigin, instruction, progress_nudge};

/// #2226: the loop's own feedback is not an instruction: a pushed feedback
/// message takes the origin of the turn already open when it is added.
#[test]
fn pushed_feedback_takes_the_origin_of_the_open_turn() {
    for (opener, origin) in [
        (progress_nudge("continue".into()), TurnOrigin::ProgressNudge),
        (instruction("task".into()), TurnOrigin::Instruction),
    ] {
        let mut messages = vec![opener, Message::assistant("", vec![])];
        assert_eq!(
            append_feedback(&mut messages, "try again".into(), 3),
            Feedback::Added
        );
        let feedback = messages.last().unwrap();
        assert_eq!(feedback.turn_origin, origin);
        assert_eq!(feedback.turn, Some(3));
    }
    let mut unstamped = vec![Message::assistant("", vec![])];
    append_feedback(&mut unstamped, "try again".into(), 1);
    assert_eq!(unstamped[1].turn_origin, TurnOrigin::Unknown);
}

#[test]
fn merged_feedback_keeps_the_trailing_messages_origin() {
    let mut messages = vec![progress_nudge("continue".into())];
    assert_eq!(
        append_feedback(&mut messages, "try again".into(), 1),
        Feedback::Merged
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].turn_origin, TurnOrigin::ProgressNudge);
}
