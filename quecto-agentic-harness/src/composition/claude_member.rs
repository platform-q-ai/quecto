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
use crate::application::external_agent::use_cases::DriveExternalAgentSession;
use crate::infrastructure::external_agents::claude_code::environment::CREDENTIAL_VARIABLES;
use crate::infrastructure::external_agents::claude_code::process::ClaudeCodeLauncher;
use crate::infrastructure::external_agents::clock::TokioExternalAgentClock;
use crate::infrastructure::external_agents::telemetry::TracingExternalAgentTelemetry;
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
    let launcher = Arc::new(ClaudeCodeLauncher::new(supervisor, parent_environment));
    let session = DriveExternalAgentSession::new(
        launcher,
        Arc::new(TracingExternalAgentTelemetry),
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
/// value, which the launcher refuses before anything runs.
fn credential_from(parent_environment: &[(OsString, OsString)]) -> Result<CredentialEnv, String> {
    let value_of = |name: &str| {
        parent_environment
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
    };
    let set: Vec<(&str, String)> = CREDENTIAL_VARIABLES
        .iter()
        .filter_map(|name| value_of(name).map(|value| (*name, value)))
        .collect();
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
