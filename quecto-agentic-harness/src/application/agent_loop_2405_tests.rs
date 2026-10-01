//! #2405: the context ceiling comes from the model's real window, less the
//! reply's reserve, and an unknown window says so once.

use super::*;
use crate::application::catalogue::dto::ModelLimits;
use crate::application::catalogue::ports::ModelRuntime;

fn agent(max_context_tokens: usize, max_tokens: u32, window: Option<usize>) -> AgentLoopImpl {
    let provider = Arc::new(MockProvider::new(vec![]));
    AgentLoopImpl::new(AgentLoopConfig {
        max_context_tokens,
        max_tokens,
        model_context_window: window,
        ..test_config(provider, Box::new(MockRegistry::new()))
    })
}

fn limits(max_output_tokens: Option<u32>, context_window: Option<usize>) -> ModelLimits {
    ModelLimits {
        max_output_tokens,
        context_window,
    }
}

/// At startup the composition builds the loop with the window and then sets
/// the declared output cap: the ceiling keeps that cap free of the window.
#[test]
fn the_startup_ceiling_keeps_the_declared_output_cap_free_of_the_window() {
    let agent = agent(300_000, 8_192, Some(400_000)).with_model_max_tokens(Some(128_000));
    assert_eq!(agent.effective_max_context_tokens(), 272_000);
    assert_eq!(
        agent.max_context_tokens(),
        272_000,
        "the reported ceiling agrees"
    );
}

#[test]
fn a_switch_recomputes_the_ceiling_from_the_new_window_and_cap() {
    let mut agent = agent(300_000, 8_192, None);
    agent.apply_model(
        "openai-api/gpt-5.3-codex".into(),
        limits(Some(128_000), Some(400_000)),
    );
    assert_eq!(agent.effective_max_context_tokens(), 272_000);

    // No declared cap: the configured reply size is the reserve.
    agent.apply_model(
        "openai-api/gpt-5.3-codex-spark".into(),
        limits(None, Some(128_000)),
    );
    assert_eq!(agent.effective_max_context_tokens(), 119_808);

    // A configured budget below the room wins.
    agent.apply_model(
        "openai-api/gpt-5.5".into(),
        limits(Some(128_000), Some(1_050_000)),
    );
    assert_eq!(agent.effective_max_context_tokens(), 300_000);

    // An unknown window falls back to the configured budget.
    agent.apply_model("acme/unknown".into(), limits(Some(128_000), None));
    assert_eq!(agent.effective_max_context_tokens(), 300_000);
}

/// An unknown window keeps today's behaviour and says so once per model,
/// not on every request or every switch back.
#[test]
fn an_unknown_window_is_noted_once_per_model() {
    let mut agent = agent(200_000, 8_192, None);
    assert_eq!(
        agent.unknown_window_notes(),
        1,
        "the startup model is noted"
    );

    agent.apply_model("test-model".into(), limits(None, None));
    assert_eq!(
        agent.unknown_window_notes(),
        1,
        "the same model is not noted again"
    );

    agent.apply_model("acme/known".into(), limits(None, Some(100_000)));
    assert_eq!(
        agent.unknown_window_notes(),
        1,
        "a known window is not noted"
    );

    agent.apply_model("acme/other".into(), limits(None, None));
    assert_eq!(
        agent.unknown_window_notes(),
        2,
        "another unknown model is noted"
    );

    agent.apply_model("test-model".into(), limits(None, None));
    assert_eq!(
        agent.unknown_window_notes(),
        2,
        "a model already noted stays quiet"
    );
}
