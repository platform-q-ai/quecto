//! Contract for the `EffortRuntime` port (#1848): the agent loop reports the
//! level it applies and accepts a new one from the use case; `None` means
//! the provider default; a session switch restores the startup default.
use quecto::application::catalogue::ports::EffortRuntime;
use quecto::domain::provider::EffortLevel;

use super::switch_runtime_fixture::runtime;

#[test]
fn applied_level_is_reported_and_none_restores_the_provider_default() {
    let mut rt = runtime("cli:effort-contract");
    assert_eq!(
        rt.agent.effort(),
        None,
        "the fixture starts on the provider default"
    );
    rt.agent.apply_effort(Some(EffortLevel::High));
    assert_eq!(EffortRuntime::effort(&rt.agent), Some(EffortLevel::High));
    rt.agent.apply_effort(None);
    assert_eq!(EffortRuntime::effort(&rt.agent), None);
}

#[test]
fn a_session_switch_restores_the_startup_default_not_the_override() {
    let mut rt = runtime("cli:effort-contract");
    rt.agent.apply_effort(Some(EffortLevel::XHigh));
    rt.agent.reset_effort_to_default();
    assert_eq!(rt.agent.effort(), None, "the fixture's startup default");
}
