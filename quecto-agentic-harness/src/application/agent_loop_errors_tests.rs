use super::append_feedback;
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

#[test]
fn provider_error_enhancement_preserves_failure_transition_and_audit() {
    use super::super::agent_loop_turn::{
        Output, ProviderFailureTransition, classify_provider_failure,
    };
    use super::{enhance_provider_error, provider_failure_audit_event};
    use crate::domain::error::DomainError;
    use crate::domain::inference::services::provider_error::{
        ProviderErrorClass, classify_provider_error, model_refusal,
    };
    for (marker, class, terminal) in [
        (
            "HTTP 400 from Codex: class=client; refusal=model",
            ProviderErrorClass::Client,
            true,
        ),
        (
            "HTTP 429 from OpenAI: class=billing; refusal=none",
            ProviderErrorClass::Billing,
            true,
        ),
        (
            "HTTP 500 from Anthropic: class=client; refusal=none; limit=context length",
            ProviderErrorClass::Client,
            true,
        ),
        (
            "HTTP 500 from OpenAI: class=client; refusal=none",
            ProviderErrorClass::Client,
            false,
        ),
    ] {
        let raw = DomainError::Provider(marker.into());
        let enhanced = enhance_provider_error(DomainError::Provider(marker.into()));
        assert_eq!(classify_provider_error(&enhanced), class);
        assert_eq!(model_refusal(&enhanced), model_refusal(&raw));
        assert_eq!(
            classify_provider_failure(&enhanced, Output::NotShown, 0, 3),
            if terminal {
                ProviderFailureTransition::Terminal(class.clone())
            } else {
                ProviderFailureTransition::RecoverMalformedRequest
            }
        );
        let audit = serde_json::to_value(provider_failure_audit_event("test", &enhanced)).unwrap();
        assert_eq!(audit["class"], class.as_str());
        assert_eq!(audit["http_status"], marker[5..8].parse::<u16>().unwrap());
    }
}
