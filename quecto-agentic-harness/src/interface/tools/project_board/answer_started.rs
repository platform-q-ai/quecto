//! What `start` answers (S4c): the `task` as written, its `run`, the
//! `coordinator` launched for it (its `agent` and the container it runs
//! in, `environment_ref`), the worker roles whose models were not applied
//! (`not_applied`: `role`, `model`, `effort`), whether the start took the
//! claim (`claim`: `fresh`) or found it held by the starter (`held`), and
//! when to renew it (`renew_after`); on the agent tool, [`super::outcome`]
//! adds the `steps` for running it ([`super::start_steps::running`]).
//!
//! A board server's begun start (epic #2705 B8) answers the same, with no
//! `coordinator` yet, and what its client launches with: the status it
//! took the task `from`, and `launch`: the `plan_version`, the coordinator's
//! `brief` as rendered, and its `model` and `effort` (`null`: the
//! session's own). The client puts its `coordinator` in their place.
use serde_json::{Map, Value, json};

use super::{fields, renew_after_json, run_json, shown, slug_json, task_json};
use crate::application::project_board::start::{Begun, StartWrite, StartedTask};
use crate::application::project_board::swarm_brief::NotApplied;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::telemetry::StartClaim;

pub(super) fn started_fields(started: &StartedTask) -> Map<String, Value> {
    let coordinator = &started.coordinator;
    let coordinator = json!({
        "agent": shown(&coordinator.agent),
        "environment_ref": shown(&coordinator.environment_ref),
    });
    let (task, run) = (&started.task, &started.run);
    let mut answer = fields([("task", task_json(task)), ("run", run_json(run))]);
    answer.insert("coordinator".into(), coordinator);
    answer.extend(claim_fields(task, &started.not_applied, started.claim));
    answer
}

pub(super) fn begun_fields(begun: &Begun<StartWrite>) -> Map<String, Value> {
    let (task, run) = (begun.written.started.task(), begun.written.started.run());
    let launch = &begun.launch;
    let mut answer = fields([("task", task_json(task)), ("run", run_json(run))]);
    answer.insert("from".into(), json!(begun.from.as_str()));
    answer.insert(
        "launch".into(),
        json!({
            "plan_version": launch.plan_version,
            "brief": launch.brief,
            "model": launch.model,
            "effort": launch.effort.map(|effort| effort.as_str()),
        }),
    );
    answer.extend(claim_fields(task, &begun.written.not_applied, begun.claim));
    answer
}

fn claim_fields(task: &Task, not_applied: &[NotApplied], claim: StartClaim) -> Map<String, Value> {
    let not_applied: Vec<Value> = not_applied
        .iter()
        .map(|role| {
            json!({
                "role": slug_json(&role.role),
                "model": shown(&role.model),
                "effort": role.effort.as_str(),
            })
        })
        .collect();
    fields([
        ("not_applied", Value::Array(not_applied)),
        ("claim", json!(claim.as_str())),
        // When to renew the claim: halfway through it (#2656). A started
        // task is held by its starter.
        ("renew_after", renew_after_json(task)),
    ])
}
