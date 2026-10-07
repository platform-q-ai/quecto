//! Typed coordination adapter. Python wire details stop at this boundary.
use super::SwarmContext;
use crate::application::swarm::ports::CoordinationPort;
use crate::domain::error::DomainError;
use crate::domain::swarm::MemberExit;
use crate::domain::swarm::{Member, MemberStatus, ProcessIdentity, RunStatus, Snapshot};
use serde::Deserialize;
use serde_json::{Value, json};

/// What a run watch's tick read (#2338): the board's event cursor, and
/// the run's snapshot unless the cursor was the one the watch passed.
#[derive(Clone, Debug)]
pub struct RunWatch {
    pub event_cursor: i64,
    pub snapshot: Option<Snapshot>,
}

#[derive(Deserialize)]
struct WireMember {
    id: String,
    status: String,
    pid: Option<u32>,
    started: Option<String>,
    socket: Option<String>,
    #[serde(default)]
    launcher: Option<String>,
}
#[derive(Deserialize)]
struct WireSnapshot {
    control_generation: u64,
    status: String,
    #[serde(default)]
    outcome: Option<String>,
    coordinator: String,
    deadline: f64,
    members: Vec<WireMember>,
}
fn invalid(message: impl std::fmt::Display) -> DomainError {
    DomainError::Tool(format!("invalid swarm coordination response: {message}"))
}
pub(super) fn decode_status(status: &str) -> Result<RunStatus, DomainError> {
    Ok(match status {
        "setup" => RunStatus::Setup,
        "running" => RunStatus::Running,
        "paused" => RunStatus::Paused,
        "succeeded" => RunStatus::Succeeded,
        "blocked" => RunStatus::Blocked,
        "failed" => RunStatus::Failed,
        "cancelled" => RunStatus::Cancelled,
        "budget-exhausted" => RunStatus::BudgetExhausted,
        _ => return Err(invalid("unknown run status")),
    })
}
fn decode(value: Value) -> Result<Snapshot, DomainError> {
    let wire: WireSnapshot = serde_json::from_value(value).map_err(invalid)?;
    let status = decode_status(&wire.status)?;
    let members = wire
        .members
        .into_iter()
        .map(decode_member)
        .collect::<Result<Vec<_>, DomainError>>()?;
    if !members.iter().any(|m| m.id == wire.coordinator) {
        return Err(invalid("coordinator missing from membership"));
    }
    let outcome = wire.outcome.as_deref().map(decode_status).transpose()?;
    Ok(Snapshot {
        control_generation: wire.control_generation,
        status,
        outcome,
        coordinator: wire.coordinator,
        deadline: wire.deadline,
        members,
    })
}
fn decode_member(m: WireMember) -> Result<Member, DomainError> {
    let status = match m.status.as_str() {
        "live" => MemberStatus::Live,
        "reserved" => MemberStatus::Reserved,
        "dead" => MemberStatus::Dead,
        _ => return Err(invalid("unknown member status")),
    };
    let process = match (m.pid, m.started) {
        (Some(pid), Some(started)) if pid > 0 && !started.is_empty() => {
            Some(ProcessIdentity { pid, started })
        }
        (None, None) => None,
        _ => return Err(invalid("incomplete process identity")),
    };
    Ok(Member {
        id: m.id,
        status,
        process,
        endpoint: m.socket,
        launcher: m.launcher,
    })
}
impl CoordinationPort for SwarmContext {
    fn snapshot(&self) -> Result<Snapshot, DomainError> {
        decode(self.rpc("_snapshot", json!([]))?)
    }
    fn register_endpoint(&self, endpoint: &str) -> Result<(), DomainError> {
        self.rpc("_socket", json!([endpoint])).map(|_| ())
    }
    fn reserve_member(&self, member: &str, token: &str) -> Result<(), DomainError> {
        self.rpc("_admit", json!([member, token])).map(|_| ())
    }
    fn record_launch(
        &self,
        member: &str,
        token: &str,
        process: &ProcessIdentity,
    ) -> Result<(), DomainError> {
        self.rpc(
            "_record_launch",
            json!([member, token, process.pid, process.started]),
        )
        .map(|_| ())
    }
    fn confirm_unlaunched(&self, member: &str) -> Result<(), DomainError> {
        self.rpc("_release_unlaunched", json!([member])).map(|_| ())
    }
    fn quarantine(&self, member: &str) -> Result<(), DomainError> {
        self.rpc("_quarantine", json!([member])).map(|_| ())
    }
    fn confirm_dead(&self, member: &str, exit: MemberExit) -> Result<(), DomainError> {
        self.rpc("_confirmed_dead", json!([member, exit.as_str()]))
            .map(|_| ())
    }
}
impl SwarmContext {
    /// The run watch's tick (#2338): one `_watch` call, recorded as the
    /// harness's own (an `unchanged` answer aggregated with the ones
    /// around it). Passing the cursor of its last snapshot, `since`, it
    /// gets only the board's cursor back while that is unchanged, and the
    /// run's snapshot with the cursor otherwise; passing none, always the
    /// snapshot.
    pub fn watch(&self, since: Option<i64>) -> Result<RunWatch, DomainError> {
        let value = self.board.call_as(
            self.location(),
            &self.member,
            "_watch",
            json!([since]),
            super::super::swarm_board_dispatch::CallOrigin::Watch,
        )?;
        let event_cursor = value["event_cursor"]
            .as_i64()
            .filter(|cursor| *cursor >= 0)
            .ok_or_else(|| invalid("missing event cursor"))?;
        let snapshot = match (value.get("snapshot"), value["unchanged"].as_bool()) {
            (Some(snapshot), None) => Some(decode(snapshot.clone())?),
            (None, Some(true)) if since == Some(event_cursor) => None,
            _ => return Err(invalid("a watch answers the snapshot or unchanged")),
        };
        Ok(RunWatch {
            event_cursor,
            snapshot,
        })
    }

    /// Writes the run watch's held polls now (#2338): the watch ended.
    pub fn flush_watch_polls(&self) {
        self.board.flush_watch_polls(&self.location());
    }

    /// The run as `gate` admits from it (#2339): the board records the
    /// read as that gate's decision.
    pub fn inference_snapshot(
        &self,
        gate: crate::domain::swarm::AdmissionGate,
    ) -> Result<Snapshot, DomainError> {
        decode(self.rpc("_request_admission", json!([gate.as_str()]))?)
    }

    pub(crate) fn decode_control_receipt(
        value: Value,
        wake_allowed: bool,
    ) -> Result<crate::domain::swarm::RunControlReceipt, DomainError> {
        Ok(crate::domain::swarm::RunControlReceipt {
            budget: value
                .get("budget")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(invalid)?,
            wake_allowed,
            status: decode_status(
                value["status"]
                    .as_str()
                    .ok_or_else(|| invalid("missing control status"))?,
            )?,
            outcome: value["outcome"].as_str().map(decode_status).transpose()?,
            reason: value["reason"].as_str().map(str::to_owned),
            wake_warnings: Vec::new(),
            resume_blockers: value["resume_blockers"]
                .as_array()
                .map(|blockers| {
                    blockers
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            generation: value["generation"]
                .as_u64()
                .ok_or_else(|| invalid("missing control generation"))?,
        })
    }

    pub fn notification_batch(&self) -> Result<(Vec<Member>, u64), DomainError> {
        let value = self.rpc("_notifications", json!([true]))?;
        let generation = value["generation"]
            .as_u64()
            .ok_or_else(|| invalid("missing wake generation"))?;
        let members: Vec<WireMember> =
            serde_json::from_value(value["members"].clone()).map_err(invalid)?;
        Ok((
            members
                .into_iter()
                .map(decode_member)
                .collect::<Result<Vec<_>, _>>()?,
            generation,
        ))
    }

    pub fn notifications(&self) -> Result<Vec<Member>, DomainError> {
        let members: Vec<WireMember> =
            serde_json::from_value(self.rpc("_notifications", json!([]))?).map_err(invalid)?;
        members.into_iter().map(decode_member).collect()
    }

    pub fn join(
        &self,
        process: &ProcessIdentity,
        endpoint: Option<&str>,
        reservation: Option<&str>,
    ) -> Result<Snapshot, DomainError> {
        decode(self.rpc(
            "_bootstrap",
            json!([process.pid, process.started, endpoint, reservation]),
        )?)
    }
    pub fn create_run(
        &self,
        input: &Value,
        process: &ProcessIdentity,
        endpoint: Option<&str>,
    ) -> Result<Snapshot, DomainError> {
        let result = self.rpc(
            "create",
            json!([
                input["goal"],
                run_constraints(input),
                input["criteria"],
                input["member_limit"],
                absolute_deadline(input)?
            ]),
        )?;
        let member = result["members"]
            .as_array()
            .and_then(|members| members.iter().find(|m| m["id"] == self.member))
            .ok_or_else(|| invalid("invoking member missing"))?;
        let reservation = member["reservation"]
            .as_str()
            .ok_or_else(|| invalid("reservation missing"))?;
        self.rpc(
            "_activate",
            json!([
                self.member,
                reservation,
                process.pid,
                process.started,
                endpoint
            ]),
        )?;
        self.snapshot()
    }
    pub fn cancel_run(&self) -> Result<(), DomainError> {
        self.rpc("stop", json!(["cancelled", "parent/user cancellation"]))
            .map(|_| ())
    }
}

#[cfg(test)]
#[path = "swarm_coordination_tests.rs"]
mod tests;

impl crate::application::providers::ports::RequestAccounting for SwarmContext {
    fn record<'a>(
        &'a self,
        observation: &'a crate::domain::inference::events::request_observation::RequestObservation,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + 'a>>
    {
        let context = self.clone();
        let observation = observation.clone();
        // On the blocking pool as blocking work (a board call), but not the
        // call's carried work: a panic in it is this record's durable error,
        // which the agent loop logs and drops, never a panic resumed in the
        // loop (#2278 final review L3).
        Box::pin(async move {
            tokio::task::spawn_blocking(crate::infrastructure::tools::call_work::blocking(
                move || {
                    let mut value = serde_json::to_value(observation).map_err(invalid)?;
                    value["runtime"] =
                        serde_json::to_value(crate::infrastructure::runtime_identity::current())
                            .map_err(invalid)?;
                    context.rpc("_record_request", json!([value])).map(|_| ())
                },
            ))
            .await
            .map_err(invalid)?
        })
    }
}

/// The run's `constraints` (#2205): optional, so an omitted list — or an
/// explicit `null`, which JSON callers send for "none" — is empty. Any
/// other value goes to the store as given, which refuses anything but a
/// list of strings — so a wrong type is still named as one.
fn run_constraints(input: &Value) -> Value {
    match input.get("constraints") {
        None | Some(Value::Null) => json!([]),
        Some(given) => given.clone(),
    }
}

/// The run's deadline as Unix seconds (#2125): now plus
/// `deadline_in_seconds` when given (it wins over `deadline`, so a
/// placeholder `deadline` never hides it), else `deadline`. A model need not
/// know the current time; the store still checks the result.
fn absolute_deadline(input: &Value) -> Result<Value, DomainError> {
    const SEVEN_DAYS: f64 = 604_800.0;
    match (input.get("deadline_in_seconds"), input.get("deadline")) {
        (Some(seconds), _) if !seconds.is_null() => {
            let seconds = seconds
                .as_f64()
                .filter(|s| *s > 0.0 && *s <= SEVEN_DAYS)
                .ok_or_else(|| {
                    invalid("deadline_in_seconds must be a number from 1 to 604800 (seven days)")
                })?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            Ok(json!(now + seconds))
        }
        (_, Some(deadline)) => Ok(deadline.clone()),
        _ => Ok(Value::Null),
    }
}

impl SwarmContext {
    /// The run as this member reads it if it is the run's coordinator
    /// (#2467): `_status`, then, for the run's coordinator only,
    /// `_run_totals`, both recorded as the harness's own reads. Blocking:
    /// call it off the async workers.
    pub(crate) fn coordinator_board_now(
        &self,
    ) -> Result<
        Option<crate::domain::swarm::parent_wake::CoordinatorBoard>,
        crate::domain::error::DomainError,
    > {
        let status = self.host_read("_status")?;
        decode_coordinator_board(&self.member, &status, || self.host_read("_run_totals"))
    }

    /// The run as its coordinator reads it when a worker ends a turn
    /// (#2471): [`Self::coordinator_board_now`], then the owner of each
    /// claimed task from `tasks`. Blocking.
    pub(crate) fn worker_board_now(
        &self,
    ) -> Result<
        Option<crate::domain::swarm::worker_wake::WorkerBoard>,
        crate::domain::error::DomainError,
    > {
        let Some(board) = self.coordinator_board_now()? else {
            return Ok(None);
        };
        // `tasks` pages by at most 100.
        const PAGE: usize = 100;
        let mut claimed_by = Vec::new();
        for offset in (0..).step_by(PAGE) {
            let page = self.board.call_as(
                self.location(),
                &self.member,
                "tasks",
                json!([offset, PAGE]),
                super::super::swarm_board_dispatch::CallOrigin::Harness,
            )?;
            claimed_by.extend(decode_claimed_owners(&page)?);
            match page.as_array().map(Vec::len) {
                Some(PAGE) => {}
                Some(_) | None => break,
            }
        }
        Ok(Some(crate::domain::swarm::worker_wake::WorkerBoard {
            status: board.status,
            ready: board.ready,
            claimed_by,
            coordinator: self.member.clone(),
        }))
    }

    fn host_read(&self, method: &str) -> Result<Value, crate::domain::error::DomainError> {
        self.board.call_as(
            self.location(),
            &self.member,
            method,
            json!([]),
            super::super::swarm_board_dispatch::CallOrigin::Harness,
        )
    }
}

/// The run as its coordinator reads it (#2467), from the board's `_status`
/// answer and, read only when `member` is the run's coordinator, its
/// `_run_totals`: `None` for any other member.
pub(crate) fn decode_coordinator_board(
    member: &str,
    status: &Value,
    totals: impl FnOnce() -> Result<Value, crate::domain::error::DomainError>,
) -> Result<
    Option<crate::domain::swarm::parent_wake::CoordinatorBoard>,
    crate::domain::error::DomainError,
> {
    let malformed = |what: &str| {
        crate::domain::error::DomainError::Tool(format!("coordinator board: {what} missing"))
    };
    let coordinator = status
        .get("coordinator")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("coordinator"))?;
    match coordinator == member {
        true => {}
        false => return Ok(None),
    }
    let text = |key: &str| {
        status
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| malformed(key))
    };
    let count = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_i64)
            .ok_or_else(|| malformed(key))
    };
    let outcome = match status.get("outcome") {
        None | Some(Value::Null) => None,
        Some(outcome) => Some(decode_status(
            outcome.as_str().ok_or_else(|| malformed("outcome"))?,
        )?),
    };
    let run_status = decode_status(text("status")?)?;
    let idle_workers = count(status, "members_without_claim")?;
    let totals = totals()?;
    let tasks = totals.get("tasks").ok_or_else(|| malformed("tasks"))?;
    Ok(Some(crate::domain::swarm::parent_wake::CoordinatorBoard {
        status: run_status,
        outcome,
        ready: count(tasks, "ready")?,
        claimed: count(tasks, "claimed")?,
        submitted: count(tasks, "submitted")?,
        idle_workers,
    }))
}

/// The owner of each claimed task in a `tasks` answer (#2471).
pub(crate) fn decode_claimed_owners(
    tasks: &Value,
) -> Result<Vec<String>, crate::domain::error::DomainError> {
    let malformed =
        |what: &str| crate::domain::error::DomainError::Tool(format!("worker board: {what}"));
    let tasks = tasks
        .as_array()
        .ok_or_else(|| malformed("tasks is not a list"))?;
    let mut owners = Vec::new();
    for task in tasks {
        match task.get("status").and_then(Value::as_str) {
            Some("claimed") => owners.push(
                task.get("owner")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("a claimed task has no owner"))?
                    .to_owned(),
            ),
            Some("ready" | "blocked" | "submitted" | "completed") => {}
            Some(_) | None => return Err(malformed("a task has no known status")),
        }
    }
    Ok(owners)
}
