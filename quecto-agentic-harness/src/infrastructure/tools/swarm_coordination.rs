//! Typed coordination adapter. Python wire details stop at this boundary.
use super::SwarmContext;
use crate::domain::error::DomainError;
use crate::domain::swarm::{
    CoordinationPort, Member, MemberStatus, ProcessIdentity, RunStatus, Snapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct WireMember {
    id: String,
    status: String,
    pid: Option<u32>,
    started: Option<String>,
    socket: Option<String>,
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
fn decode_status(status: &str) -> Result<RunStatus, DomainError> {
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
}
impl SwarmContext {
    pub fn inference_snapshot(&self) -> Result<Snapshot, DomainError> {
        decode(self.rpc("_request_admission", json!([]))?)
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
                input["constraints"],
                input["criteria"],
                input["member_limit"],
                input["deadline"]
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

impl crate::domain::request_observation::RequestAccounting for SwarmContext {
    fn record<'a>(
        &'a self,
        observation: &'a crate::domain::request_observation::RequestObservation,
    ) -> crate::domain::subagent_launch::LaunchFuture<'a, Result<(), DomainError>> {
        let context = self.clone();
        let observation = observation.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut value = serde_json::to_value(observation).map_err(invalid)?;
                value["runtime"] =
                    serde_json::to_value(crate::infrastructure::runtime_identity::current())
                        .map_err(invalid)?;
                context.rpc("_record_request", json!([value])).map(|_| ())
            })
            .await
            .map_err(invalid)?
        })
    }
}
