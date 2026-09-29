//! A member session's records as event-log events (#2304): the
//! `external_agent_*` events of [`AuditEvent`], filed through the
//! [`crate::application::audit::ports::AuditSink`] every quecto agent
//! writes its log through. Every [`SessionRecord`] is placed: logged, or
//! deliberately left to `tracing` alone (a prompt's admission, carried by
//! the turn it starts; a tool's call, logged once answered).
#![allow(dead_code, unused_imports)] // red-phase stub (#2304)

use super::dto::SessionRecord;
use crate::domain::audit::AuditEvent;
use crate::domain::external_agent::telemetry::ExternalAgentLifecycle;

/// Who the records are of: the member's ref on the board, and the mode of
/// the credential it runs under (never the credential).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberIdentity {
    pub member_ref: String,
    pub credential_mode: &'static str,
}

/// `record` as the event it is logged as, with the turn it is filed
/// under; `None` for a record the log leaves to `tracing`.
pub fn audit_event(record: &SessionRecord, member: &MemberIdentity) -> Option<(u32, AuditEvent)> {
    let _ = (record, member);
    None
}

/// The envelope's turn: the member's, saturated to its width; 0 for none.
fn filed(turn: Option<u64>) -> u32 {
    turn.map_or(0, |turn| u32::try_from(turn).unwrap_or(u32::MAX))
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;
