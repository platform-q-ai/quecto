//! The claude-code member runner (#2287): `cmd_agent` routes `--backend
//! claude-code` here after admission; the runner holds composition's
//! handles only.

use super::super::agent::{cmd_agent, parse_agent_flags};
use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

const NOT_COMPOSED: &str = "agent: the claude-code member capability is not composed\n";

#[test]
fn an_uncomposed_member_capability_is_refused() {
    let mut stderr = String::new();
    let flags = parse_agent_flags(
        &argv(&["--mode", "uds", "--backend", "claude-code"]),
        &mut stderr,
    )
    .expect("valid flags");
    assert_eq!(run(&CliContext::default(), &flags, &mut stderr), 1);
    assert_eq!(stderr, NOT_COMPOSED);
}

/// #2287 review (L3): outside a swarm run, `cmd_agent` refuses the
/// claude-code backend after admission, before the runner.
#[test]
fn cmd_agent_refuses_a_claude_code_member_outside_a_swarm() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        cwd: Some(tmp.path().to_path_buf()),
        claude_member: Some(|_| panic!("the runner is never reached")),
        ..Default::default()
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = cmd_agent(
        &ctx,
        &argv(&["--mode", "uds", "--backend", "claude-code", "-s", "w1"]),
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(
        (code, stderr.as_str()),
        (
            1,
            "agent: --backend claude-code runs only as a swarm worker admitted to a created swarm run\n"
        )
    );
}

/// #2287 review (L1): a member composition refuses to build is refused
/// with its reason, before anything runs.
#[test]
fn a_member_composition_refuses_is_refused_with_its_reason() {
    let mut stderr = String::new();
    let flags = parse_agent_flags(
        &argv(&["--mode", "uds", "--backend", "claude-code"]),
        &mut stderr,
    )
    .expect("valid flags");
    let ctx = CliContext {
        claude_member: Some(|_| Err("no credential for you".to_string())),
        ..Default::default()
    };
    assert_eq!(run(&ctx, &flags, &mut stderr), 1);
    assert_eq!(stderr, "agent: no credential for you\n");
}

/// Fakes of the member session's ports: a launcher that refuses or starts
/// a process that never speaks. The real claude CLI never runs here.
mod fakes {
    use std::sync::Arc;
    use std::time::Duration;

    use super::super::handles::{ClaudeMemberHandles, ClaudeMemberSettings};
    use crate::application::external_agent::dto::{
        AgentClockInstant, CredentialEnv, ExternalAgentExit, ExternalAgentInputError,
        ExternalAgentLaunchError, ExternalAgentLaunchSpec, ExternalAgentSessionSettings,
        SessionRecord,
    };
    use crate::application::external_agent::ports::{
        DetachedWork, ExternalAgentClock, ExternalAgentLauncher, ExternalAgentProcess,
        ExternalAgentSpawner, ExternalAgentTelemetry, PortFuture, QueuedUserTurn,
    };
    use crate::application::external_agent::use_cases::DriveExternalAgentSession;
    use crate::domain::external_agent::stream::ExternalAgentEvent;

    struct Silent;

    impl ExternalAgentProcess for Silent {
        fn queue_user_turn<'a>(
            &'a self,
            _text: &'a str,
        ) -> PortFuture<'a, Result<QueuedUserTurn<'a>, ExternalAgentInputError>> {
            Box::pin(std::future::pending())
        }
        fn interrupt(&self) -> PortFuture<'_, Result<(), ExternalAgentInputError>> {
            Box::pin(std::future::pending())
        }
        fn next_event(&self) -> PortFuture<'_, Option<ExternalAgentEvent>> {
            Box::pin(std::future::pending())
        }
        fn close_input(&self) -> PortFuture<'_, ()> {
            Box::pin(async {})
        }
        fn exited(&self) -> PortFuture<'_, ExternalAgentExit> {
            Box::pin(std::future::pending())
        }
        // Its input closed, it exits at once.
        fn exited_discarding_output(&self) -> PortFuture<'_, ExternalAgentExit> {
            Box::pin(async { ExternalAgentExit::Code(0) })
        }
        fn stderr_tail(&self) -> String {
            String::new()
        }
    }

    struct Launcher {
        refuse: bool,
    }

    impl ExternalAgentLauncher for Launcher {
        fn start<'a>(
            &'a self,
            _spec: ExternalAgentLaunchSpec,
        ) -> PortFuture<'a, Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError>>
        {
            Box::pin(async move {
                match self.refuse {
                    true => Err(ExternalAgentLaunchError::NotFound {
                        program: "claude".into(),
                        required_for: "claude-code members".into(),
                    }),
                    false => Ok(Box::new(Silent) as Box<dyn ExternalAgentProcess>),
                }
            })
        }
    }

    struct Unrecorded;

    impl ExternalAgentTelemetry for Unrecorded {
        fn record(&self, _record: &SessionRecord) {}
        fn finish(&self) -> PortFuture<'_, ()> {
            Box::pin(async {})
        }
    }

    /// Runs detached work as a tokio task of the test's runtime.
    struct Tasks;

    impl ExternalAgentSpawner for Tasks {
        fn spawn(&self, work: DetachedWork) {
            drop(tokio::spawn(work));
        }
    }

    struct Frozen;

    impl ExternalAgentClock for Frozen {
        fn now(&self) -> AgentClockInstant {
            AgentClockInstant(0)
        }
        fn sleep(&self, _duration: Duration) -> PortFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }

    fn member(refuse: bool) -> ClaudeMemberHandles {
        ClaudeMemberHandles {
            session: Arc::new(DriveExternalAgentSession::new(
                Arc::new(Launcher { refuse }),
                Arc::new(Unrecorded),
                Arc::new(Frozen),
                Arc::new(Tasks),
                ExternalAgentSessionSettings {
                    launch: ExternalAgentLaunchSpec {
                        model: "claude-sonnet".into(),
                        tools: Vec::new(),
                        mcp_config: serde_json::json!({}),
                        settings: serde_json::json!({}),
                        max_budget_usd: 1.0,
                        checkout: "/work".into(),
                        member_dir: "/members/w1".into(),
                        credential: CredentialEnv {
                            name: "ANTHROPIC_API_KEY".into(),
                            value: "sk-ant-api03-SECRETSECRETSECRET".into(),
                        },
                    },
                    skipped_line_grace: Duration::from_secs(60),
                    interrupt_grace: Duration::from_secs(30),
                },
            )),
        }
    }

    /// A member whose agent starts, and never speaks.
    pub(super) fn starting(_: &ClaudeMemberSettings) -> Result<ClaudeMemberHandles, String> {
        Ok(member(false))
    }

    /// A member whose agent cannot be started.
    pub(super) fn refused(_: &ClaudeMemberSettings) -> Result<ClaudeMemberHandles, String> {
        Ok(member(true))
    }
}

fn claude_code_flags() -> AgentFlags {
    let mut stderr = String::new();
    let flags = parse_agent_flags(
        &argv(&["--mode", "uds", "--backend", "claude-code", "-s", "w1"]),
        &mut stderr,
    )
    .expect("valid flags");
    assert_eq!(stderr, "");
    flags
}

/// This slice's runner starts the member and ends it again (its endpoint
/// lands with #2288): the run fails, saying so.
#[test]
fn a_started_member_is_ended_again_and_the_run_says_why() {
    let ctx = CliContext {
        claude_member: Some(fakes::starting),
        ..Default::default()
    };
    let mut stderr = String::new();
    assert_eq!(run(&ctx, &claude_code_flags(), &mut stderr), 1);
    assert_eq!(stderr, NO_ENDPOINT_YET);
}

/// A member whose agent cannot start is refused with the launch's reason.
#[test]
fn a_member_whose_agent_cannot_start_is_refused_with_its_reason() {
    let ctx = CliContext {
        claude_member: Some(fakes::refused),
        ..Default::default()
    };
    let mut stderr = String::new();
    assert_eq!(run(&ctx, &claude_code_flags(), &mut stderr), 1);
    assert_eq!(
        stderr,
        "agent: claude not found on PATH (required for claude-code members)\n"
    );
}

/// The handles' `Debug` names them and shows nothing of the session.
#[test]
fn the_member_handles_debug_shows_nothing_of_the_session() {
    let handles = fakes::starting(&ClaudeMemberSettings {
        member: "w1".into(),
        model: None,
        checkout: "/work".into(),
        base_dir: "/base".into(),
        parent: None,
        config_path: None,
        log_key: None,
    })
    .expect("built");
    assert_eq!(format!("{handles:?}"), "ClaudeMemberHandles { .. }");
}

fn log_key_for(args: &[&str]) -> Option<String> {
    let mut stderr = String::new();
    let mut parts = vec!["--mode", "uds", "--backend", "claude-code"];
    parts.extend_from_slice(args);
    let flags = parse_agent_flags(&argv(&parts), &mut stderr).expect("valid flags");
    member_settings(&CliContext::default(), &flags).log_key
}

/// #2304 review round 3: a member asked to leave nothing behind
/// (`--no-session`, `-s -`) keeps no event log.
#[test]
fn a_member_asked_to_leave_nothing_behind_keeps_no_log() {
    assert_eq!(log_key_for(&["--no-session"]), None);
    assert_eq!(log_key_for(&["-s", "-"]), None);
}

/// #2304 review round 3: a named member's log is its session's; an
/// unnamed one's is its own, never another unnamed member's.
#[test]
fn each_unnamed_member_keeps_a_log_of_its_own() {
    assert_eq!(log_key_for(&["-s", "w1"]).as_deref(), Some("cli:w1"));
    let unnamed = log_key_for(&[]).expect("an unnamed member keeps a log");
    let own = format!("unkeyed-{}-", std::process::id());
    assert!(unnamed.starts_with(&own), "{unnamed}");
    assert_ne!(unnamed, "cli:claude-member");
}
