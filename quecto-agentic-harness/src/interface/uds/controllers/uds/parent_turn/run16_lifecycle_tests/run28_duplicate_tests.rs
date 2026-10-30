//! Transaction-entry rollback through the real composed AgentLoop policy port.
use super::*;
use crate::domain::tool_policy::value_objects::tool::{
    ToolPolicyApplyMode, ToolPolicyMutationStatus,
};
use crate::domain::tool_policy::value_objects::tool_descriptor::ToolCatalogueEntry;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug)]
enum EntryState {
    Enabled,
    Disabled,
    Mixed,
}

fn definitions_of(rig: &Rig) -> Vec<(String, String, String)> {
    rig.env
        .agent
        .tool_definitions()
        .iter()
        .map(|definition| {
            (
                definition.name.to_string(),
                definition.description.to_string(),
                definition.parameters_schema.to_string(),
            )
        })
        .collect()
}

async fn prepare_entry_state(rig: &mut Rig, state: EntryState) -> String {
    let read_id = rig
        .env
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "read")
        .unwrap()
        .stable_id
        .into_owned();
    match state {
        EntryState::Enabled => {}
        EntryState::Disabled | EntryState::Mixed => {
            rig.policy(false, false).await;
            let scope = match state {
                EntryState::Mixed => ProfileAvailabilityScope::Both,
                _ => ProfileAvailabilityScope::None,
            };
            assert!(
                rig.env
                    .agent
                    .apply_persisted_tool_policy_entries(
                        &[(read_id.clone(), scope)].into_iter().collect()
                    )
                    .is_empty()
            );
        }
    }
    read_id
}

fn duplicate_mutations(state: EntryState, read_id: String) -> Vec<ToolPolicyMutation> {
    match state {
        // The exact review reproducer: an originally enabled read ends enabled.
        EntryState::Enabled => vec![
            ToolPolicyMutation::disable("read", "first duplicate"),
            ToolPolicyMutation::enable("read", "second duplicate"),
        ],
        // Resolved aliases, repeated no-op, and interleaved independent key.
        EntryState::Disabled | EntryState::Mixed => vec![
            ToolPolicyMutation::enable(read_id, "alias enable"),
            ToolPolicyMutation::disable("write", "other key"),
            ToolPolicyMutation::enable("read", "repeated enable"),
            ToolPolicyMutation::disable("read", "duplicate disable"),
            ToolPolicyMutation::enable("read", "final enable"),
        ],
    }
}

fn assert_entry_state(before: &[ToolCatalogueEntry], state: EntryState) {
    let read = before.iter().find(|entry| entry.name == "read").unwrap();
    match state {
        EntryState::Enabled => assert!(read.effective_parent_enabled),
        EntryState::Disabled => {
            assert_eq!(read.profile_scope, Some(ProfileAvailabilityScope::None));
            assert!(!read.runtime_availability.is_enabled());
        }
        EntryState::Mixed => {
            assert_eq!(read.profile_scope, Some(ProfileAvailabilityScope::Both));
            assert!(!read.runtime_availability.is_enabled());
        }
    }
}

async fn transaction(state: EntryState, queued: bool, fail: bool) {
    let mut rig = Rig::new(vec![]);
    let read_id = prepare_entry_state(&mut rig, state).await;
    let before = rig.env.agent.tool_catalogue_entries();
    assert_entry_state(&before, state);
    let definitions = definitions_of(&rig);
    let visibility = ["read", "write", "declare_turn_outcome"]
        .map(|name| rig.env.agent.is_tool_model_visible(name));
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    rig.env
        .agent
        .set_tool_policy_persistence(Some(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            if fail {
                Err("run28 durable failure".into())
            } else {
                Ok(())
            }
        })));
    let mutations = duplicate_mutations(state, read_id);
    let count = mutations.len();
    let mut request = ToolPolicyRequest::patch(mutations);
    request.persist = true;
    request.correlation_id = Some("run28-transaction".into());
    let mode = if queued {
        ToolPolicyApplyMode::AtNextTurnBoundary
    } else {
        ToolPolicyApplyMode::ImmediateIfIdle
    };
    let immediate = rig.env.agent.request_tool_policy(request, mode);
    let result = if queued {
        assert!(immediate.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            rig.env.agent.tool_catalogue_entries(),
            before,
            "queued request cannot mutate entry state before boundary"
        );
        assert_eq!(definitions_of(&rig), definitions);
        rig.env
            .agent
            .drain_tool_policy_mutations_at_boundary()
            .unwrap()
    } else {
        immediate.unwrap()
    };
    assert_eq!(result.mode, mode);
    assert_eq!(result.correlation_id.as_deref(), Some("run28-transaction"));
    assert_eq!(result.results.len(), count);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        rig.env
            .agent
            .drain_tool_policy_mutations_at_boundary()
            .is_none(),
        "success or failure consumes the queued transaction exactly once"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let after = rig.env.agent.tool_catalogue_entries();
    println!("RUN28 state={state:?} queued={queued} fail={fail} mutations={count}");
    if fail {
        for item in &result.results {
            assert_eq!(item.status, ToolPolicyMutationStatus::PersistenceFailed);
            assert!(
                item.reason
                    .ends_with("; persisted tool policy update failed: run28 durable failure")
            );
        }
        assert_eq!(
            after, before,
            "failed duplicate transaction must restore EXACT entry state, not intermediate snapshots"
        );
        assert_eq!(definitions_of(&rig), definitions);
        assert_eq!(
            ["read", "write", "declare_turn_outcome"]
                .map(|name| rig.env.agent.is_tool_model_visible(name)),
            visibility,
            "failure preserves existing model-visibility overrides"
        );
    } else {
        assert!(result.results.iter().all(|item| matches!(
            item.status,
            ToolPolicyMutationStatus::Applied | ToolPolicyMutationStatus::AlreadyInState
        )));
        let read = after.iter().find(|entry| entry.name == "read").unwrap();
        assert!(read.effective_parent_enabled);
        assert!(read.runtime_availability.is_enabled());
        assert_eq!(read.profile_scope, Some(ProfileAvailabilityScope::Both));
        assert_eq!(read.configured_enabled, Some(true));
        assert!(rig.env.agent.is_tool_model_visible("read"));
        if matches!(state, EntryState::Disabled | EntryState::Mixed) {
            let write = after.iter().find(|entry| entry.name == "write").unwrap();
            assert!(!write.effective_parent_enabled);
            assert_eq!(write.configured_enabled, Some(false));
        }
    }
}

macro_rules! failure_case {
    ($name:ident, $state:ident, $queued:expr) => {
        #[tokio::test]
        async fn $name() {
            if isolated(concat!(module_path!(), "::", stringify!($name))) {
                transaction(EntryState::$state, $queued, true).await;
            }
        }
    };
}
failure_case!(run28_enabled_immediate_failure, Enabled, false);
failure_case!(run28_enabled_queued_failure, Enabled, true);
failure_case!(run28_disabled_immediate_failure, Disabled, false);
failure_case!(run28_disabled_queued_failure, Disabled, true);
failure_case!(run28_mixed_immediate_failure, Mixed, false);
failure_case!(run28_mixed_queued_failure, Mixed, true);

#[tokio::test]
async fn run28_immediate_success_controls() {
    if isolated(concat!(
        module_path!(),
        "::run28_immediate_success_controls"
    )) {
        for state in [EntryState::Enabled, EntryState::Disabled, EntryState::Mixed] {
            transaction(state, false, false).await;
        }
    }
}

#[tokio::test]
async fn run28_queued_success_controls() {
    if isolated(concat!(module_path!(), "::run28_queued_success_controls")) {
        for state in [EntryState::Enabled, EntryState::Disabled, EntryState::Mixed] {
            transaction(state, true, false).await;
        }
    }
}
