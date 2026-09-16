//! Contract for the `ModelRuntime` port (#1847): the agent loop reports the
//! model it runs on and switches model and limits together, re-clamping
//! the effective output cap and context budget; the change-active-model use
//! case is its only caller.
use quecto::application::catalogue::dto::ModelLimits;
use quecto::application::catalogue::ports::ModelRuntime;

use super::switch_runtime_fixture::runtime;

#[test]
fn model_and_limits_switch_together_and_reclamp() {
    let mut rt = runtime("cli:model-contract");
    assert_eq!(rt.agent.model(), "stub");
    let before = rt.agent.effective_max_tokens();
    assert!(
        before > 8,
        "the fixture's configured cap exceeds the clamp under test"
    );
    rt.agent.apply_model(
        "acme/limited".into(),
        ModelLimits {
            max_output_tokens: Some(8),
            context_window: Some(2_048),
        },
    );
    assert_eq!(ModelRuntime::model(&rt.agent), "acme/limited");
    assert_eq!(rt.agent.effective_max_tokens(), 8);
    assert_eq!(rt.agent.effective_max_context_tokens(), 2_048);
    // No declared limits lifts the clamp back to the configured values.
    rt.agent
        .apply_model("acme/open".into(), ModelLimits::default());
    assert_eq!(rt.agent.effective_max_tokens(), before);
    assert!(rt.agent.effective_max_context_tokens() > 2_048);
}
