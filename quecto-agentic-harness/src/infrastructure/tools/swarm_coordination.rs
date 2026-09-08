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
    status: String,
    coordinator: String,
    deadline: f64,
    members: Vec<WireMember>,
}
fn invalid(message: impl std::fmt::Display) -> DomainError {
    DomainError::Tool(format!("invalid swarm coordination response: {message}"))
}
fn decode(value: Value) -> Result<Snapshot, DomainError> {
    let wire: WireSnapshot = serde_json::from_value(value).map_err(invalid)?;
    let status = match wire.status.as_str() {
        "setup" => RunStatus::Setup,
        "running" => RunStatus::Running,
        "succeeded" => RunStatus::Succeeded,
        "blocked" => RunStatus::Blocked,
        "failed" => RunStatus::Failed,
        "cancelled" => RunStatus::Cancelled,
        "budget-exhausted" => RunStatus::BudgetExhausted,
        _ => return Err(invalid("unknown run status")),
    };
    let members = wire
        .members
        .into_iter()
        .map(decode_member)
        .collect::<Result<Vec<_>, DomainError>>()?;
    if !members.iter().any(|m| m.id == wire.coordinator) {
        return Err(invalid("coordinator missing from membership"));
    }
    Ok(Snapshot {
        status,
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
