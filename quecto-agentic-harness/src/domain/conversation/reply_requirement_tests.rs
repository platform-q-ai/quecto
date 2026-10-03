use super::*;
use crate::domain::message::ToolCall;

fn call(id: &str) -> Message {
    Message::assistant(
        "",
        vec![ToolCall {
            id: id.to_string(),
            name: "background".to_string(),
            arguments: "{}".to_string(),
        }],
    )
}

#[test]
fn a_reply_to_a_prompt_must_have_output() {
    let messages = [Message::system("sys"), Message::user("question")];
    assert_eq!(
        ReplyRequirement::for_conversation(&messages),
        ReplyRequirement::Output
    );
}

#[test]
fn a_reply_to_tool_results_may_be_empty() {
    let messages = [
        Message::system("sys"),
        Message::user("start the job"),
        call("c1"),
        Message::tool("c1", "started — end your turn now"),
    ];
    assert_eq!(
        ReplyRequirement::for_conversation(&messages),
        ReplyRequirement::MayBeEmpty
    );
}

#[test]
fn a_reply_to_several_tool_results_may_be_empty() {
    let mut calls = call("c1");
    calls.tool_calls.extend(call("c2").tool_calls);
    let messages = [
        Message::user("start both"),
        calls,
        Message::tool("c1", "one"),
        Message::tool("c2", "two"),
    ];
    assert_eq!(
        ReplyRequirement::for_conversation(&messages),
        ReplyRequirement::MayBeEmpty
    );
}

#[test]
fn a_steer_or_follow_up_after_tool_results_must_be_answered() {
    let messages = [
        Message::user("start the job"),
        call("c1"),
        Message::tool("c1", "started"),
        Message::user("actually, also tell me how long it takes"),
    ];
    assert_eq!(
        ReplyRequirement::for_conversation(&messages),
        ReplyRequirement::Output
    );
}

#[test]
fn a_system_message_after_tool_results_changes_nothing() {
    let messages = [
        Message::user("start the job"),
        call("c1"),
        Message::tool("c1", "started"),
        Message::system("[Session memory is available via recall()]"),
    ];
    assert_eq!(
        ReplyRequirement::for_conversation(&messages),
        ReplyRequirement::MayBeEmpty
    );
}

#[test]
fn a_conversation_with_nothing_to_answer_requires_output() {
    assert_eq!(
        ReplyRequirement::for_conversation(&[]),
        ReplyRequirement::Output
    );
    assert_eq!(
        ReplyRequirement::for_conversation(&[Message::system("sys")]),
        ReplyRequirement::Output
    );
}
