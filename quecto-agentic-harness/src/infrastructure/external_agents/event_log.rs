//! A member session's records as event-log events (#2304): the
//! `external_agent_*` events of [`AuditEvent`] the event-log telemetry
//! adapter ([`super::telemetry::EventLogExternalAgentTelemetry`]) files
//! through the [`crate::application::audit::ports::AuditSink`] every quecto
//! agent writes its log through. Every [`SessionRecord`] is placed: logged, or
//! deliberately left to `tracing` alone (a prompt's admission, carried by
//! the turn it starts; a tool's call, logged once answered).

use crate::application::external_agent::dto::SessionRecord;
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
    let lifecycle = |turn: Option<u64>, record: ExternalAgentLifecycle| {
        Some((
            filed(turn),
            AuditEvent::ExternalAgentLifecycle {
                member_ref: member.member_ref.clone(),
                record,
            },
        ))
    };
    match record {
        SessionRecord::TurnReported(turn) => Some((
            filed(Some(turn.member_turn)),
            AuditEvent::ExternalAgentTurn {
                member_ref: member.member_ref.clone(),
                record: turn.clone(),
            },
        )),
        SessionRecord::ToolFinished(tool) => Some((
            filed(tool.member_turn),
            AuditEvent::ExternalAgentTool {
                member_ref: member.member_ref.clone(),
                record: tool.clone(),
            },
        )),
        SessionRecord::StreamDiagnostic(diagnostic) => Some((
            filed(diagnostic.member_turn),
            AuditEvent::ExternalAgentStreamDiagnostic {
                member_ref: member.member_ref.clone(),
                record: diagnostic.clone(),
            },
        )),
        SessionRecord::Started => lifecycle(
            None,
            ExternalAgentLifecycle::Started {
                credential_mode: member.credential_mode.to_string(),
            },
        ),
        SessionRecord::StartRefused { kind } => lifecycle(
            None,
            ExternalAgentLifecycle::StartRefused {
                reason: kind.to_string(),
            },
        ),
        SessionRecord::Initialized {
            cli_version,
            claude_session_id,
            model,
        } => lifecycle(
            None,
            ExternalAgentLifecycle::Initialized {
                cli_version: cli_version.clone(),
                claude_session_id: claude_session_id.clone(),
                model: model.clone(),
            },
        ),
        SessionRecord::Interrupted { turn, cause } => lifecycle(
            Some(*turn),
            ExternalAgentLifecycle::Interrupted {
                member_turn: *turn,
                cause: cause.to_string(),
            },
        ),
        SessionRecord::Aborted {
            turn,
            dropped_follow_ups,
        } => lifecycle(
            *turn,
            ExternalAgentLifecycle::Aborted {
                member_turn: *turn,
                dropped_follow_ups: *dropped_follow_ups,
            },
        ),
        SessionRecord::Abandoned {
            turn,
            dropped_follow_ups,
        } => lifecycle(
            Some(*turn),
            ExternalAgentLifecycle::Abandoned {
                member_turn: *turn,
                dropped_follow_ups: *dropped_follow_ups,
            },
        ),
        SessionRecord::PromptRefused { refusal, bytes: _ } => lifecycle(
            None,
            ExternalAgentLifecycle::PromptRefused {
                refusal: refusal.to_string(),
            },
        ),
        SessionRecord::FollowUpFailed {
            turn,
            bytes: _,
            refusal,
        } => lifecycle(
            Some(*turn),
            ExternalAgentLifecycle::FollowUpFailed {
                member_turn: *turn,
                refusal: refusal.to_string(),
            },
        ),
        SessionRecord::Closed {
            turn,
            dropped_follow_ups,
        } => lifecycle(
            *turn,
            ExternalAgentLifecycle::Closed {
                member_turn: *turn,
                dropped_follow_ups: *dropped_follow_ups,
            },
        ),
        SessionRecord::Ended {
            clean,
            exit_code,
            signal,
            wall_ms,
        } => lifecycle(
            None,
            ExternalAgentLifecycle::Ended {
                clean: *clean,
                exit_code: *exit_code,
                signal: *signal,
                wall_ms: *wall_ms,
            },
        ),
        // Carried by the records above: a turn's end by `TurnReported`, a
        // call by `ToolFinished`, a skipped line by `StreamDiagnostic`; a
        // prompt's and a follow-up's admission by the turn they start.
        SessionRecord::PromptAccepted { .. }
        | SessionRecord::ToolCalled { .. }
        | SessionRecord::LineSkipped { .. }
        | SessionRecord::TurnEnded { .. }
        | SessionRecord::TurnContinued { .. }
        | SessionRecord::ResultWithoutIds { .. }
        | SessionRecord::FollowUpStarted { .. } => None,
    }
}

/// The envelope's turn: the member's, saturated to its width; 0 for none.
fn filed(turn: Option<u64>) -> u32 {
    turn.map_or(0, |turn| turn.try_into().unwrap_or(u32::MAX))
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;
