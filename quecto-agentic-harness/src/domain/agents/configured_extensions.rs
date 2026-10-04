//! Configured extensions (#2446): the UDS extensions an agent launches from
//! its global configuration, one instance per agent. A top-level agent
//! launches every configured extension; a locally spawned child launches
//! those marked `children` (each instance gets the child's own
//! `{agent_id}`, so it keeps its own state, e.g. its own browser).
//!
//! Pure policy: placeholder expansion, which agent launches what, whether
//! an exited extension is restarted and how its state reads as a warning.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// The placeholders an extension's `args` and `env` values may use. An
/// allowlist: any other `{name}` is a configuration error.
pub const PLACEHOLDERS: [&str; 3] = ["socket", "agent_id", "state_dir"];

/// The `{agent_id}` of a top-level agent.
pub const TOP_LEVEL_AGENT_ID: &str = "main";

/// How long an agent waits for a launched extension's `register_tools`
/// before it warns and takes work without it.
pub const REGISTRATION_WAIT: Duration = Duration::from_secs(30);

/// Restarts allowed per [`RESTART_WINDOW`]; one more exit stops it.
pub const MAX_RESTARTS: usize = 5;
pub const RESTART_WINDOW: Duration = Duration::from_secs(600);
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// One configured extension: a command the agent launches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSpec {
    pub name: String,
    /// An absolute path.
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Whether a locally spawned child launches its own instance.
    pub children: bool,
}

/// Which agent is launching: the top-level one, or a locally spawned
/// child (a container child launches none, so it has no role here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    TopLevel,
    LocalChild,
}

impl AgentRole {
    /// The role of an agent started with or without `--spawned`.
    pub fn of(spawned: bool) -> Self {
        match spawned {
            true => Self::LocalChild,
            false => Self::TopLevel,
        }
    }

    /// The agent's `{agent_id}`: `main` at the top, a child's own session
    /// name (its id) below.
    pub fn agent_id(self, session_name: Option<&str>) -> String {
        match (self, session_name) {
            (Self::LocalChild, Some(name)) => name.to_string(),
            (Self::TopLevel, _) | (Self::LocalChild, None) => TOP_LEVEL_AGENT_ID.to_string(),
        }
    }
}

/// The extensions one agent launches, and the `{agent_id}` it gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentExtensions {
    pub agent_id: String,
    pub specs: Vec<ExtensionSpec>,
}

impl AgentExtensions {
    /// The `configured` extensions an agent in `role` launches; none when
    /// launching is switched off (`--no-extensions`).
    pub fn select(
        configured: Vec<ExtensionSpec>,
        role: AgentRole,
        session_name: Option<&str>,
        launching: bool,
    ) -> Self {
        Self {
            agent_id: role.agent_id(session_name),
            specs: configured
                .into_iter()
                .filter(|spec| launching && spec.launches_for(role))
                .collect(),
        }
    }
}

/// The values the placeholders stand for in one agent.
#[derive(Debug, Clone, Copy)]
pub struct PlaceholderValues<'a> {
    pub socket: &'a str,
    pub agent_id: &'a str,
    pub state_dir: &'a str,
}

impl PlaceholderValues<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        match name {
            "socket" => Some(self.socket),
            "agent_id" => Some(self.agent_id),
            "state_dir" => Some(self.state_dir),
            _ => None,
        }
    }
}

impl ExtensionSpec {
    /// Whether an agent in `role` launches this extension.
    pub fn launches_for(&self, role: AgentRole) -> bool {
        match role {
            AgentRole::TopLevel => true,
            AgentRole::LocalChild => self.children,
        }
    }

    /// This extension with every placeholder in its `args` and `env` values
    /// replaced by `values`.
    pub fn expanded(&self, values: &PlaceholderValues<'_>) -> Result<Self, String> {
        let expand = |template: &String| substitute(template, |name| values.get(name));
        Ok(Self {
            args: self.args.iter().map(expand).collect::<Result<_, _>>()?,
            env: self
                .env
                .iter()
                .map(|(key, value)| Ok((key.clone(), expand(value)?)))
                .collect::<Result<_, String>>()?,
            ..self.clone()
        })
    }
}

/// Whether every placeholder in `template` is one of [`PLACEHOLDERS`].
pub fn check_placeholders(template: &str) -> Result<(), String> {
    substitute(template, |name| PLACEHOLDERS.contains(&name).then_some("")).map(|_| ())
}

/// Replace each `{name}` (a name of ASCII letters, digits and `_`) with
/// `lookup(name)`; a name `lookup` does not know is an error. Any other
/// brace is literal text, so JSON in an argument stays as written.
fn substitute<'a>(
    template: &str,
    lookup: impl Fn(&str) -> Option<&'a str>,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name = after.find('}').map(|close| &after[..close]).filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        });
        match name {
            Some(name) => {
                let value = lookup(name).ok_or_else(|| {
                    format!(
                        "unknown placeholder {{{name}}}; the placeholders are {{socket}}, {{agent_id}} and {{state_dir}}"
                    )
                })?;
                out.push_str(value);
                rest = &after[name.len() + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// How a launched extension ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionExit {
    Code(i32),
    Signal(i32),
    Unobservable,
}

impl std::fmt::Display for ExtensionExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit code {code}"),
            Self::Signal(signal) => write!(f, "signal {signal}"),
            Self::Unobservable => f.write_str("an exit that could not be observed"),
        }
    }
}

/// Why an extension is not restarted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Exit code 2: its command line is wrong; a restart repeats it.
    CommandLineError,
    /// Exit code 3: the agent refused its tools; a restart repeats it.
    ToolsRefused,
    /// It used up [`MAX_RESTARTS`] within [`RESTART_WINDOW`].
    RestartLimit,
    /// It could not be started at all.
    LaunchFailed(String),
}

/// What follows an extension's exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterExit {
    /// Start it again after `delay`; this is restart number `restart`.
    Restart {
        delay: Duration,
        restart: usize,
    },
    Stop(StopReason),
}

/// The restarts of one extension within the sliding window.
#[derive(Debug, Default)]
pub struct RestartBudget {
    restarts: VecDeque<Instant>,
}

impl RestartBudget {
    /// Decide what follows `exit` at `now`: exit codes 2 and 3 are never
    /// restarted; any other exit is, with doubling backoff, until the
    /// window's budget is spent.
    pub fn after_exit(&mut self, exit: &ExtensionExit, now: Instant) -> AfterExit {
        match exit {
            ExtensionExit::Code(2) => AfterExit::Stop(StopReason::CommandLineError),
            ExtensionExit::Code(3) => AfterExit::Stop(StopReason::ToolsRefused),
            ExtensionExit::Code(_) | ExtensionExit::Signal(_) | ExtensionExit::Unobservable => {
                while self
                    .restarts
                    .front()
                    .is_some_and(|at| now.saturating_duration_since(*at) >= RESTART_WINDOW)
                {
                    self.restarts.pop_front();
                }
                if self.restarts.len() >= MAX_RESTARTS {
                    return AfterExit::Stop(StopReason::RestartLimit);
                }
                self.restarts.push_back(now);
                let restart = self.restarts.len();
                let doubling = 1u32 << (restart - 1).min(16);
                AfterExit::Restart {
                    delay: FIRST_BACKOFF.saturating_mul(doubling).min(MAX_BACKOFF),
                    restart,
                }
            }
        }
    }
}

/// Where a configured extension stands, as `get_state` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionState {
    /// Launched; its `register_tools` has not arrived yet.
    Starting,
    /// Its tools are registered.
    Running,
    /// Running, but no `register_tools` within [`REGISTRATION_WAIT`].
    Unregistered,
    /// Exited; restart number `restart` is scheduled.
    Restarting { exit: ExtensionExit, restart: usize },
    /// Not running and not restarted.
    Stopped {
        exit: Option<ExtensionExit>,
        reason: StopReason,
    },
}

impl ExtensionState {
    /// Whether the agent's start-up wait for this extension is over: its
    /// tools arrived, the wait ran out, or it will not run.
    pub fn settled(&self) -> bool {
        match self {
            Self::Running | Self::Unregistered | Self::Stopped { .. } => true,
            Self::Starting | Self::Restarting { .. } => false,
        }
    }

    /// The warning this state shows in `startupWarnings`; none while it is
    /// starting or running.
    pub fn warning(&self, name: &str, log: &str) -> Option<String> {
        let what = match self {
            Self::Starting | Self::Running => return None,
            Self::Unregistered => format!(
                "did not register its tools within {} s; it is still running",
                REGISTRATION_WAIT.as_secs()
            ),
            Self::Restarting { exit, restart } => {
                format!("exited ({exit}); restart {restart} of {MAX_RESTARTS}")
            }
            Self::Stopped { exit, reason } => stopped(exit.as_ref(), reason),
        };
        Some(format!("extension `{name}` {what} (log: {log})"))
    }
}

fn stopped(exit: Option<&ExtensionExit>, reason: &StopReason) -> String {
    let exit = exit.map_or_else(String::new, |exit| format!(" ({exit})"));
    match reason {
        StopReason::CommandLineError => {
            "exited with code 2 (command-line error) and is not restarted".to_string()
        }
        StopReason::ToolsRefused => {
            "exited with code 3 (its tools were refused) and is not restarted".to_string()
        }
        StopReason::RestartLimit => format!(
            "exited{exit} after {MAX_RESTARTS} restarts within {} minutes and is not restarted again",
            RESTART_WINDOW.as_secs() / 60
        ),
        StopReason::LaunchFailed(error) => format!("could not be launched: {error}"),
    }
}

#[cfg(test)]
#[path = "configured_extensions_tests.rs"]
mod tests;
