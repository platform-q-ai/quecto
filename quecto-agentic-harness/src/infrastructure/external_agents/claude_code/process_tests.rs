use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

use super::test_rig::{BOUND, RESULT_LINE, Rig, events_to_turn_end, finish};
#[cfg(target_os = "linux")]
use super::test_rig::{FALLBACK_BOUND, recorded_at_start, until_retired};
use super::{claude_arguments, resolve_on_path};
use crate::application::external_agent::dto::{
    CredentialEnv, ExternalAgentExit, ExternalAgentLaunchError, ExternalAgentLaunchSpec,
};
use crate::application::external_agent::ports::ExternalAgentLauncher;
use crate::domain::external_agent::stream::ExternalAgentEvent;
#[cfg(target_os = "linux")]
use crate::infrastructure::processes::owned_child_supervisor::SentSignal;

#[tokio::test]
async fn argv_carries_the_stream_json_flags() {
    let rig = Rig::new("");
    let process = rig.start().await;
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    let expected: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        "claude-haiku-4-5",
        "--tools",
        "Read,Edit,Bash",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--strict-mcp-config",
        "--settings",
        r#"{"hooks":{}}"#,
        "--setting-sources",
        "project",
        "--permission-mode",
        "bypassPermissions",
        "--allow-dangerously-skip-permissions",
        "--no-session-persistence",
        "--max-budget-usd",
        "1.5",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    assert_eq!(rig.mock.recorded("arg"), expected);
    assert_eq!(claude_arguments(&rig.spec()).unwrap(), expected);
}

#[tokio::test]
async fn the_child_runs_in_the_checkout_with_only_the_member_environment() {
    let rig = Rig::new("");
    let process = rig.start().await;
    finish(process.as_ref()).await;
    assert_eq!(
        rig.mock.recorded("cwd"),
        vec![rig.checkout().display().to_string()]
    );
    let env = rig.mock.recorded("env");
    let names: Vec<&str> = env
        .iter()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        // The shell itself sets these.
        .filter(|name| !["PWD", "OLDPWD", "SHLVL", "_"].contains(name))
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec![
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CONFIG_DIR",
            "HOME",
            "LANG",
            "PATH"
        ],
        "{env:?}"
    );
    let member = rig.root.path().join("members/m1");
    assert!(env.contains(&format!("HOME={}", member.join("home").display())));
    assert!(env.contains(&format!(
        "CLAUDE_CONFIG_DIR={}",
        member.join("claude-config").display()
    )));
    assert!(env.contains(&"CLAUDE_CODE_OAUTH_TOKEN=member-token".to_string()));
}

#[tokio::test]
async fn a_line_that_is_not_json_is_skipped_and_the_stream_goes_on() {
    let rig = Rig::new(concat!(
        "this is not json\n",
        "[1, 2]\n",
        "{\"type\": \"system\", \"subtype\": \"thinking_tokens\", \"estimated_tokens\": 5}\n",
        "{\"type\": \"result\", \"subtype\": \"success\"}\n",
    ));
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    let events = events_to_turn_end(process.as_ref()).await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        events[0],
        ExternalAgentEvent::ThinkingTokens {
            estimated_tokens: Some(5)
        }
    );
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    let after = tokio::time::timeout(BOUND, process.next_event())
        .await
        .expect("the stream's end is bounded");
    assert_eq!(after, None, "the stream ends with the process's output");
}

#[tokio::test]
async fn an_invalid_spec_is_refused_before_anything_runs() {
    let rig = Rig::new("");
    let base = rig.spec();
    let invalid: Vec<(&str, ExternalAgentLaunchSpec)> = vec![
        (
            "empty model",
            ExternalAgentLaunchSpec {
                model: String::new(),
                ..base.clone()
            },
        ),
        (
            "flag-like model",
            ExternalAgentLaunchSpec {
                model: "--dangerously-skip-permissions".into(),
                ..base.clone()
            },
        ),
        (
            "odd tool",
            ExternalAgentLaunchSpec {
                tools: vec!["Read,Bash".into()],
                ..base.clone()
            },
        ),
        (
            "mcp config not an object",
            ExternalAgentLaunchSpec {
                mcp_config: json!([]),
                ..base.clone()
            },
        ),
        (
            "settings not an object",
            ExternalAgentLaunchSpec {
                settings: json!("x"),
                ..base.clone()
            },
        ),
        (
            "NaN budget",
            ExternalAgentLaunchSpec {
                max_budget_usd: f64::NAN,
                ..base.clone()
            },
        ),
        (
            "zero budget",
            ExternalAgentLaunchSpec {
                max_budget_usd: 0.0,
                ..base.clone()
            },
        ),
        (
            "unknown credential name",
            ExternalAgentLaunchSpec {
                credential: CredentialEnv {
                    name: "GH_TOKEN".into(),
                    value: "v".into(),
                },
                ..base.clone()
            },
        ),
        (
            "empty credential value",
            ExternalAgentLaunchSpec {
                credential: CredentialEnv {
                    name: "ANTHROPIC_API_KEY".into(),
                    value: String::new(),
                },
                ..base.clone()
            },
        ),
        (
            "relative checkout",
            ExternalAgentLaunchSpec {
                checkout: PathBuf::from("checkout"),
                ..base.clone()
            },
        ),
        (
            "missing checkout",
            ExternalAgentLaunchSpec {
                checkout: rig.root.path().join("absent"),
                ..base.clone()
            },
        ),
    ];
    for (case, spec) in invalid {
        let outcome = rig.launcher.start(spec).await;
        assert!(
            matches!(outcome, Err(ExternalAgentLaunchError::InvalidSpec(_))),
            "{case}: {:?}",
            outcome.err()
        );
    }
    assert!(!rig.mock.args_out.exists(), "nothing was spawned");
    assert!(
        !rig.root.path().join("members").exists(),
        "no member directory was made for a refused spec"
    );
}

#[tokio::test]
async fn dropping_the_process_ends_and_forgets_its_child() {
    let rig = Rig::new("");
    let process = rig.start().await;
    assert_eq!(
        rig.supervisor.slot_count(),
        1,
        "the supervisor owns the child"
    );
    drop(process);
    tokio::time::timeout(BOUND, async {
        while rig.supervisor.slot_count() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the dropped process's child ends and is retired");
}

#[test]
fn only_an_executable_file_in_an_absolute_path_dir_is_claude() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let plain = root.path().join("plain");
    let dir_named = root.path().join("dir-named");
    let real = root.path().join("real");
    for dir in [&plain, &dir_named, &real] {
        std::fs::create_dir(dir).unwrap();
    }
    std::fs::write(plain.join("claude"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(plain.join("claude"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::create_dir(dir_named.join("claude")).unwrap();
    crate::infrastructure::test_support::executable::write_executable(
        &real.join("claude"),
        "#!/bin/sh\n",
    );
    let path = std::env::join_paths([
        PathBuf::from("relative"),
        plain.clone(),
        dir_named.clone(),
        real.clone(),
    ])
    .unwrap();
    assert_eq!(resolve_on_path(&path, "claude"), Some(real.join("claude")));
    let without = std::env::join_paths([plain, dir_named]).unwrap();
    assert_eq!(resolve_on_path(&without, "claude"), None);
}

#[test]
fn the_process_outlives_the_runtime_that_started_it() {
    let rig = Rig::new(RESULT_LINE);
    let starter = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let process = starter.block_on(rig.start());
    drop(starter);
    let reader = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    reader.block_on(async {
        process.send_user_turn("go").await.unwrap();
        let events = events_to_turn_end(process.as_ref()).await;
        assert!(
            matches!(events.last(), Some(ExternalAgentEvent::Result(_))),
            "{events:?}"
        );
        assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    });
}

#[tokio::test]
async fn the_child_leads_its_own_process_group() {
    let rig = Rig::new("");
    let process = rig.start().await;
    finish(process.as_ref()).await;
    let pid = rig.mock.recorded("pid");
    assert_eq!(pid.len(), 1, "{pid:?}");
    assert_eq!(rig.mock.recorded("pgid"), pid, "pgid == pid");
}

#[tokio::test]
async fn the_private_dirs_are_owner_only_before_the_child_runs() {
    let rig = Rig::new("");
    let process = rig.start().await;
    finish(process.as_ref()).await;
    assert_eq!(rig.mock.recorded("home_mode"), vec!["drwx------"]);
    assert_eq!(rig.mock.recorded("config_mode"), vec!["drwx------"]);
}

/// Whether `pid` is gone: no longer in `/proc`, or a zombie left for
/// whichever ancestor reaps it.
#[cfg(target_os = "linux")]
fn dead(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
        Err(_) => true,
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_a_child_that_ignores_eof_and_term_kills_its_group() {
    let rig = Rig::new("@stubborn\n");
    let process = rig.start().await;
    let handles = rig.supervisor.handles();
    assert_eq!(handles.len(), 1);
    let grandchild: u32 = recorded_at_start(&rig.mock, "grandchild")
        .await
        .parse()
        .unwrap();
    assert!(!dead(grandchild), "the grandchild runs");
    drop(process);
    until_retired(&rig.supervisor, FALLBACK_BOUND).await;
    assert_eq!(
        rig.supervisor.signals_sent(handles[0]),
        vec![SentSignal::Term, SentSignal::Kill],
        "the input's close and TERM were not enough; KILL ended the group"
    );
    tokio::time::timeout(FALLBACK_BOUND, async {
        while !dead(grandchild) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the grandchild in the child's group is dead");
}

#[test]
fn an_argv_that_would_carry_the_credential_is_refused() {
    // argv is world-readable through procfs: a credential (and S5's bridge
    // tokens) must never be inlined into `--mcp-config` or `--settings`.
    let rig = Rig::new("");
    let base = rig.spec();
    for (case, spec) in [
        (
            "mcp config",
            ExternalAgentLaunchSpec {
                mcp_config: json!({"mcpServers": {"bridge": {"headers": {
                    "Authorization": "Bearer member-token"
                }}}}),
                ..base.clone()
            },
        ),
        (
            "settings",
            ExternalAgentLaunchSpec {
                settings: json!({"env": {"TOKEN": "member-token"}}),
                ..base.clone()
            },
        ),
    ] {
        assert!(
            matches!(
                claude_arguments(&spec),
                Err(ExternalAgentLaunchError::InvalidSpec(_))
            ),
            "{case}"
        );
    }
    let argv = claude_arguments(&base).unwrap();
    assert!(
        argv.iter().all(|arg| !arg.contains(&base.credential.value)),
        "{argv:?}"
    );
}

#[test]
fn a_spawn_that_finds_no_checkout_is_not_a_missing_claude() {
    let rig = Rig::new("");
    let program = rig.mock.bin_dir.join("claude");
    let not_found = || std::io::Error::from(std::io::ErrorKind::NotFound);
    let gone = rig.root.path().join("removed-checkout");
    match super::spawn_error(not_found(), &program, &gone) {
        ExternalAgentLaunchError::InvalidSpec(detail) => {
            assert!(detail.contains("removed-checkout"), "{detail}")
        }
        other => panic!("a missing checkout: {other:?}"),
    }
    assert!(matches!(
        super::spawn_error(not_found(), &program, &rig.checkout()),
        ExternalAgentLaunchError::NotFound { .. }
    ));
    assert!(matches!(
        super::spawn_error(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            &program,
            &rig.checkout()
        ),
        ExternalAgentLaunchError::Spawn(_)
    ));
}
