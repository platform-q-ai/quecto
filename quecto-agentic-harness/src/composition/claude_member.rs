//! Claude-code member composition (#2287): the member session use case
//! over the claude process adapter, the only place either is constructed.
//! `main` hands [`build_claude_member_handles`] to the CLI through
//! `CliComposition.claude_member`; the interface only invokes the handles
//! it receives.

use std::ffi::OsString;
use std::sync::Arc;

use crate::application::external_agent::dto::{
    CredentialEnv, ExternalAgentLaunchSpec, ExternalAgentSessionSettings, INTERRUPT_GRACE,
    SKIPPED_LINE_GRACE,
};
use crate::application::external_agent::ports::ExternalAgentTelemetry;
use crate::application::external_agent::use_cases::DriveExternalAgentSession;
use crate::domain::external_agent::telemetry::recorded_name;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::config::telemetry::{enabled_in, globally_enabled};
use crate::infrastructure::external_agents::claude_code::environment::CREDENTIAL_VARIABLES;
use crate::infrastructure::external_agents::claude_code::process::ClaudeCodeLauncher;
use crate::infrastructure::external_agents::clock::TokioExternalAgentClock;
use crate::infrastructure::external_agents::event_log::MemberIdentity;
use crate::infrastructure::external_agents::telemetry::{
    EventLogExternalAgentTelemetry, TracingExternalAgentTelemetry,
};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor;
use crate::interface::cli::claude_member::{ClaudeMemberHandles, ClaudeMemberSettings};

/// Where members' private state lives, under quecto's base directory.
pub const CLAUDE_MEMBERS_DIR: &str = "claude-members";

/// The model a member runs when `--model` is not given (the CLI's alias).
/// Interim: the `members.claude_code` config section (#2293) sets it.
pub const DEFAULT_CLAUDE_MODEL: &str = "sonnet";

/// The spend cap within one turn, in US dollars. Interim: #2290 sets it
/// from the board's budget.
pub const DEFAULT_TURN_BUDGET_USD: f64 = 5.0;

/// A claude-code member's handles over this process's environment and
/// supervisor, or why none can be built.
pub fn build_claude_member_handles(
    settings: &ClaudeMemberSettings,
) -> Result<ClaudeMemberHandles, String> {
    build_over(
        settings,
        std::env::vars_os().collect(),
        OwnedChildSupervisor::process_wide(),
    )
}

/// The handles over `parent_environment` (where `claude` and the credential
/// are looked up) and `supervisor`.
pub(crate) fn build_over(
    settings: &ClaudeMemberSettings,
    parent_environment: Vec<(OsString, OsString)>,
    supervisor: Arc<OwnedChildSupervisor>,
) -> Result<ClaudeMemberHandles, String> {
    let credential = credential_from(&parent_environment)?;
    let telemetry = telemetry_for(settings, &parent_environment, &credential);
    let launcher = Arc::new(ClaudeCodeLauncher::new(supervisor, parent_environment));
    let session = DriveExternalAgentSession::new(
        launcher,
        telemetry,
        Arc::new(TokioExternalAgentClock::new()),
        ExternalAgentSessionSettings {
            launch: launch_spec(settings, credential),
            skipped_line_grace: SKIPPED_LINE_GRACE,
            interrupt_grace: INTERRUPT_GRACE,
        },
    );
    Ok(ClaudeMemberHandles {
        session: Arc::new(session),
    })
}

/// The session's telemetry: `tracing` always, and the member's event log
/// when `telemetry.event_log` is on (#2304, owner decision T1: off unless
/// configured), in the owner's global config or the `--config` given. A
/// log that cannot be opened is a warning: the member runs unrecorded.
fn telemetry_for(
    settings: &ClaudeMemberSettings,
    parent_environment: &[(OsString, OsString)],
    credential: &CredentialEnv,
) -> Arc<dyn ExternalAgentTelemetry> {
    let enabled = globally_enabled(&settings.base_dir)
        || settings.config_path.as_deref().is_some_and(enabled_in);
    match enabled {
        true => event_log_telemetry(settings, parent_environment, credential),
        false => Arc::new(TracingExternalAgentTelemetry),
    }
}

/// The session's telemetry into the member's event log, or `tracing` alone
/// when the log cannot be opened.
fn event_log_telemetry(
    settings: &ClaudeMemberSettings,
    parent_environment: &[(OsString, OsString)],
    credential: &CredentialEnv,
) -> Arc<dyn ExternalAgentTelemetry> {
    let member = MemberIdentity {
        member_ref: member_ref(parent_environment, &settings.member),
        credential_mode: credential_mode(credential),
    };
    let key = SessionIdentity::named_cli(&settings.member).map_or_else(
        |_| settings.member.clone(),
        |id| id.runtime_key().to_string(),
    );
    let opened = AuditLog::open_sync(&settings.base_dir, &key)
        .map_err(|error| error.to_string())
        .and_then(|log| {
            let log = log.with_parent(settings.parent.clone());
            EventLogExternalAgentTelemetry::new(Arc::new(log), member)
                .map_err(|error| error.to_string())
        });
    match opened {
        Ok(telemetry) => Arc::new(telemetry),
        Err(error) => {
            tracing::warn!(%error, "claude member event log could not be opened; the member runs unrecorded");
            Arc::new(TracingExternalAgentTelemetry)
        }
    }
}

/// The member's ref on the board: the swarm's `QUECTO_SWARM_MEMBER`, else
/// its session name.
fn member_ref(parent_environment: &[(OsString, OsString)], member: &str) -> String {
    parent_environment
        .iter()
        .rev()
        .find(|(name, _)| name == SWARM_MEMBER_VARIABLE)
        .and_then(|(_, value)| value.to_str())
        .filter(|value| !value.is_empty())
        .map_or_else(|| recorded_name(member), recorded_name)
}

/// The variable a swarm names its member by.
const SWARM_MEMBER_VARIABLE: &str = "QUECTO_SWARM_MEMBER";

/// The mode of the credential a member runs under: never its value.
fn credential_mode(credential: &CredentialEnv) -> &'static str {
    match (credential.name.as_str(), credential.value.is_empty()) {
        (_, true) => "none",
        ("CLAUDE_CODE_OAUTH_TOKEN", false) => "oauth_token",
        ("ANTHROPIC_API_KEY", false) => "api_key",
        (_, false) => "other",
    }
}

fn launch_spec(
    settings: &ClaudeMemberSettings,
    credential: CredentialEnv,
) -> ExternalAgentLaunchSpec {
    let model = settings
        .model
        .as_deref()
        .map_or(DEFAULT_CLAUDE_MODEL, |model| {
            // quecto names a model `provider/model`; the CLI takes the model.
            model.strip_prefix("anthropic/").unwrap_or(model)
        });
    ExternalAgentLaunchSpec {
        model: model.to_string(),
        // Every tool off, no MCP server and no settings until #2291 and
        // #2289 give the member its own.
        tools: Vec::new(),
        mcp_config: serde_json::json!({"mcpServers": {}}),
        settings: serde_json::json!({}),
        max_budget_usd: DEFAULT_TURN_BUDGET_USD,
        checkout: settings.checkout.clone(),
        member_dir: settings
            .base_dir
            .join(CLAUDE_MEMBERS_DIR)
            .join(&settings.member),
        credential,
    }
}

/// The one credential variable the environment holds a value for. Interim
/// until #2293 selects the mode by config: with both set, which one the
/// member runs under would be a guess, so it is refused, naming both
/// variables and neither value; with none, the first is named without a
/// value, which the launcher refuses before anything runs. A value that is
/// not UTF-8 is refused, naming its variable and never its value: a lossy
/// conversion would hand claude a credential nobody set.
fn credential_from(parent_environment: &[(OsString, OsString)]) -> Result<CredentialEnv, String> {
    let value_of = |name: &str| -> Result<Option<String>, String> {
        let value = parent_environment
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.to_str());
        match value {
            None => Ok(None),
            Some(Some(value)) if !value.is_empty() => Ok(Some(value.to_string())),
            Some(Some(_)) => Ok(None),
            Some(None) => Err(format!(
                "{name} is not valid UTF-8; a claude-code member takes a UTF-8 credential"
            )),
        }
    };
    let mut set: Vec<(&str, String)> = Vec::new();
    for &name in CREDENTIAL_VARIABLES {
        if let Some(value) = value_of(name)? {
            set.push((name, value));
        }
    }
    match set.as_slice() {
        [] => Ok(CredentialEnv {
            name: CREDENTIAL_VARIABLES[0].to_string(),
            value: String::new(),
        }),
        [(name, value)] => Ok(CredentialEnv {
            name: name.to_string(),
            value: value.clone(),
        }),
        [_, _, ..] => Err(format!(
            "both {} are set; a claude-code member takes exactly one until #2293 selects it by \
             config: unset one",
            CREDENTIAL_VARIABLES.join(" and ")
        )),
    }
}

#[cfg(test)]
#[path = "claude_member_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "claude_member_telemetry_tests.rs"]
mod telemetry_tests;
