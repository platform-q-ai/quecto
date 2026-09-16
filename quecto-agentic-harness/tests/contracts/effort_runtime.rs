//! Contract for the `EffortRuntime` port (#1848): the agent loop reports the
//! level it applies and the level it started with, and accepts a new one
//! from the use case; `None` means the provider default. Nothing but the
//! use case writes the running level.
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
fn the_startup_level_is_reported_separately_from_the_running_one() {
    let mut rt = runtime("cli:effort-contract");
    assert_eq!(
        rt.agent.startup_effort(),
        None,
        "the fixture's startup default"
    );
    rt.agent.apply_effort(Some(EffortLevel::XHigh));
    assert_eq!(
        rt.agent.startup_effort(),
        None,
        "an override never rewrites the startup level"
    );
    assert_eq!(EffortRuntime::effort(&rt.agent), Some(EffortLevel::XHigh));
}
