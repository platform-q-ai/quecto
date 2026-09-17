//! `ReloadRuntimeConfiguration` against fake ports: a forced reload always
//! rebuilds and reports failure; a poll rebuilds only on change and keeps
//! the last-good runtime silently on failure; a success applies provider
//! and tool policy from one rebuilt configuration. The rebuild phase needs
//! no runtime borrow, so a caller can run it off its scheduler and apply
//! the step afterwards.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

#[derive(Debug)]
struct NamedProvider(&'static str);

impl LlmProvider for NamedProvider {
    fn name(&self) -> &str {
        self.0
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        unreachable!("never called")
    }
}

type ScriptedPolicy = Vec<(&'static str, ProfileAvailabilityScope)>;

#[derive(Default)]
struct Script {
    changed: Vec<bool>,
    rebuilds: Vec<Result<(&'static str, ScriptedPolicy), String>>,
    changed_calls: usize,
    rebuild_calls: usize,
}

#[derive(Clone, Default)]
struct FakeSource(Arc<Mutex<Script>>);

impl FakeSource {
    fn changing(changed: bool) -> Self {
        let fake = Self::default();
        fake.0.lock().unwrap().changed = vec![changed];
        fake
    }
    fn will_rebuild(self, provider: &'static str, policy: ScriptedPolicy) -> Self {
        self.0.lock().unwrap().rebuilds.push(Ok((provider, policy)));
        self
    }
    fn will_fail(self, error: &str) -> Self {
        self.0.lock().unwrap().rebuilds.push(Err(error.to_string()));
        self
    }
    fn rebuild_calls(&self) -> usize {
        self.0.lock().unwrap().rebuild_calls
    }
    fn changed_calls(&self) -> usize {
        self.0.lock().unwrap().changed_calls
    }
}

impl RuntimeConfigurationSource for FakeSource {
    fn changed(&mut self) -> bool {
        let mut script = self.0.lock().unwrap();
        script.changed_calls += 1;
        script.changed.remove(0)
    }
    fn rebuild(&mut self) -> Result<ReloadedConfiguration, String> {
        let mut script = self.0.lock().unwrap();
        script.rebuild_calls += 1;
        let (provider, policy) = script.rebuilds.remove(0)?;
        Ok(ReloadedConfiguration {
            provider: Arc::new(NamedProvider(provider)),
            tool_policy: policy
                .into_iter()
                .map(|(id, scope)| (id.to_string(), scope))
                .collect(),
        })
    }
}

/// Records what it was told; knows only the `alpha` tool.
#[derive(Default)]
struct FakeRuntime {
    providers: Vec<String>,
    policies: Vec<HashMap<String, ProfileAvailabilityScope>>,
}

impl ReloadRuntime for FakeRuntime {
    fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.providers.push(provider.name().to_string());
    }
    fn apply_persisted_tool_policy(
        &mut self,
        entries: &HashMap<String, ProfileAvailabilityScope>,
    ) -> Vec<String> {
        assert_eq!(
            self.providers.len(),
            self.policies.len() + 1,
            "the policy baseline is re-applied after the provider swap, from the same rebuild"
        );
        self.policies.push(entries.clone());
        let mut unknown: Vec<String> = entries
            .keys()
            .filter(|id| id.as_str() != "alpha")
            .cloned()
            .collect();
        unknown.sort();
        unknown
    }
}

fn use_case(source: &FakeSource) -> ReloadRuntimeConfiguration {
    ReloadRuntimeConfiguration::new(Box::new(source.clone()))
}

#[test]
fn an_unconfigured_run_reports_not_configured_on_both_triggers() {
    let uc = ReloadRuntimeConfiguration::unconfigured();
    let mut runtime = FakeRuntime::default();
    assert_eq!(
        uc.apply(&mut runtime, uc.rebuild()),
        ReloadOutcome::NotConfigured
    );
    assert_eq!(
        uc.apply(&mut runtime, uc.rebuild_if_changed()),
        ReloadOutcome::NotConfigured
    );
    assert!(runtime.providers.is_empty());
    assert!(format!("{uc:?}").contains("configured: false"));
}

#[test]
fn a_forced_reload_rebuilds_without_asking_whether_anything_changed() {
    let source = FakeSource::default().will_rebuild("v2", vec![]);
    let mut runtime = FakeRuntime::default();
    assert_eq!(
        forced(&source, &mut runtime),
        ReloadOutcome::Reloaded {
            unknown_policy_tools: vec![]
        }
    );
    assert_eq!(source.changed_calls(), 0);
    assert_eq!(source.rebuild_calls(), 1);
    assert_eq!(runtime.providers, vec!["v2"]);
}

#[test]
fn a_forced_reload_reports_the_rebuild_error_and_swaps_nothing() {
    let source = FakeSource::default().will_fail("malformed config");
    let mut runtime = FakeRuntime::default();
    assert_eq!(
        forced(&source, &mut runtime),
        ReloadOutcome::Failed("malformed config".into())
    );
    assert!(runtime.providers.is_empty());
    assert!(runtime.policies.is_empty());
}

#[test]
fn a_poll_with_no_change_neither_rebuilds_nor_swaps() {
    let source = FakeSource::changing(false).will_rebuild("never", vec![]);
    let mut runtime = FakeRuntime::default();
    assert_eq!(polled(&source, &mut runtime), ReloadOutcome::Unchanged);
    assert_eq!(source.rebuild_calls(), 0);
    assert!(runtime.providers.is_empty());
}

#[test]
fn a_poll_after_a_change_rebuilds_and_swaps_the_new_provider() {
    let source = FakeSource::changing(true).will_rebuild("v2", vec![]);
    let mut runtime = FakeRuntime::default();
    assert_eq!(
        polled(&source, &mut runtime),
        ReloadOutcome::Reloaded {
            unknown_policy_tools: vec![]
        }
    );
    assert_eq!(runtime.providers, vec!["v2"]);
}

#[test]
fn a_poll_whose_rebuild_fails_keeps_the_last_good_runtime_and_reports_unchanged() {
    let source = FakeSource::changing(true).will_fail("malformed config");
    let mut runtime = FakeRuntime::default();
    assert_eq!(polled(&source, &mut runtime), ReloadOutcome::Unchanged);
    assert_eq!(source.rebuild_calls(), 1);
    assert!(runtime.providers.is_empty());
}

/// Deliberate fix (a): the tool-policy baseline comes from the same rebuilt
/// configuration as the provider — one read per reload, never a second
/// parse — and the unknown ids are reported on the outcome.
#[test]
fn a_successful_reload_applies_the_tool_policy_from_the_same_rebuild_and_reports_unknown_ids() {
    let source = FakeSource::default().will_rebuild(
        "v2",
        vec![
            ("alpha", ProfileAvailabilityScope::None),
            ("ghost", ProfileAvailabilityScope::Child),
        ],
    );
    let mut runtime = FakeRuntime::default();
    let outcome = forced(&source, &mut runtime);
    assert_eq!(
        outcome,
        ReloadOutcome::Reloaded {
            unknown_policy_tools: vec!["ghost".into()]
        }
    );
    assert_eq!(
        source.rebuild_calls(),
        1,
        "one rebuild feeds both applications"
    );
    assert_eq!(runtime.policies.len(), 1);
    assert_eq!(
        runtime.policies[0].get("alpha"),
        Some(&ProfileAvailabilityScope::None)
    );
    assert_eq!(
        runtime.policies[0].get("ghost"),
        Some(&ProfileAvailabilityScope::Child)
    );
}

/// The two phases compose: a rebuild step needs no runtime, and applying
/// it later swaps exactly what the step carried — so an interface can run
/// the rebuild off its scheduler and apply on the dispatch task.
#[test]
fn a_rebuild_step_is_applied_later_without_a_second_rebuild() {
    let source = FakeSource::default().will_rebuild("v2", vec![]);
    let uc = use_case(&source);
    let step = uc.rebuild();
    assert!(matches!(&step, ReloadStep::Ready(ready) if ready.provider.name() == "v2"));
    assert_eq!(source.rebuild_calls(), 1);
    let mut runtime = FakeRuntime::default();
    assert!(runtime.providers.is_empty(), "nothing swapped before apply");
    assert_eq!(
        uc.apply(&mut runtime, step),
        ReloadOutcome::Reloaded {
            unknown_policy_tools: vec![]
        }
    );
    assert_eq!(source.rebuild_calls(), 1);
    assert_eq!(runtime.providers, vec!["v2"]);
}

#[test]
fn rebuild_steps_carry_the_trigger_policy_for_failure_and_absence() {
    let mut runtime = FakeRuntime::default();
    let unconfigured = ReloadRuntimeConfiguration::unconfigured();
    assert!(matches!(unconfigured.rebuild(), ReloadStep::NotConfigured));
    assert!(matches!(
        unconfigured.rebuild_if_changed(),
        ReloadStep::NotConfigured
    ));
    assert_eq!(
        unconfigured.apply(&mut runtime, ReloadStep::NotConfigured),
        ReloadOutcome::NotConfigured
    );

    let forced = use_case(&FakeSource::default().will_fail("bad"));
    let step = forced.rebuild();
    assert!(matches!(&step, ReloadStep::Failed(error) if error == "bad"));
    assert_eq!(
        forced.apply(&mut runtime, step),
        ReloadOutcome::Failed("bad".into())
    );

    let polled = use_case(&FakeSource::changing(true).will_fail("bad"));
    assert!(matches!(polled.rebuild_if_changed(), ReloadStep::Unchanged));
    let quiet = use_case(&FakeSource::changing(false));
    assert!(matches!(quiet.rebuild_if_changed(), ReloadStep::Unchanged));
    assert_eq!(
        quiet.apply(&mut runtime, ReloadStep::Unchanged),
        ReloadOutcome::Unchanged
    );
    assert!(runtime.providers.is_empty());
}

/// The two phases as production composes them (`uds_dispatch_reload.rs`).
fn forced(source: &FakeSource, runtime: &mut FakeRuntime) -> ReloadOutcome {
    let uc = use_case(source);
    let step = uc.rebuild();
    uc.apply(runtime, step)
}

fn polled(source: &FakeSource, runtime: &mut FakeRuntime) -> ReloadOutcome {
    let uc = use_case(source);
    let step = uc.rebuild_if_changed();
    uc.apply(runtime, step)
}
