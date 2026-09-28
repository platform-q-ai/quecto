use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::{ClaudeCodeLauncher, claude_arguments, resolve_on_path};
use crate::application::external_agent::dto::{
    CredentialEnv, ExternalAgentExit, ExternalAgentLaunchError, ExternalAgentLaunchSpec,
};
use crate::application::external_agent::ports::{ExternalAgentLauncher, ExternalAgentProcess};
use crate::domain::external_agent::stream::ExternalAgentEvent;
use crate::infrastructure::processes::owned_child_supervisor::{
    OwnedChildSupervisor, SentSignal, TerminationBudget,
};
use crate::infrastructure::test_support::mock_claude::{MockClaude, write_mock_claude};

const BOUND: Duration = Duration::from_secs(20);

/// A short fallback, so a child that ignores its input's close (and TERM)
/// is ended within [`FALLBACK_BOUND`] rather than the default's seconds.
const FAST: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_millis(200),
    term_grace: Duration::from_millis(300),
    kill_grace: Duration::from_secs(2),
};

/// Well inside [`TerminationBudget::DEFAULT`]'s 14 s, well outside `FAST`.
const FALLBACK_BOUND: Duration = Duration::from_secs(6);

/// A turn's worth of scenario: one `result` line.
const RESULT_LINE: &str = "{\"type\": \"result\", \"subtype\": \"success\"}\n";

struct Rig {
    root: tempfile::TempDir,
    mock: MockClaude,
    supervisor: Arc<OwnedChildSupervisor>,
    launcher: ClaudeCodeLauncher,
}

impl Rig {
    fn new(scenario: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let scenario_path = root.path().join("scenario.jsonl");
        std::fs::write(&scenario_path, scenario).unwrap();
        let mock = write_mock_claude(&root.path().join("bin"), &scenario_path);
        std::fs::create_dir(root.path().join("checkout")).unwrap();
        let supervisor = Arc::new(OwnedChildSupervisor::new());
        let path = format!("{}:/usr/bin:/bin", mock.bin_dir.display());
        let parent = [
            ("PATH", path.as_str()),
            ("LANG", "C.UTF-8"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "parent-session"),
            ("ANTHROPIC_BASE_URL", "http://leak.invalid"),
            ("GH_TOKEN", "leak"),
            ("QUECTO_BASE_DIR", "/leak"),
        ]
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();
        let launcher =
            ClaudeCodeLauncher::new(Arc::clone(&supervisor), parent).with_termination_budget(FAST);
        Self {
            root,
            mock,
            supervisor,
            launcher,
        }
    }

    fn checkout(&self) -> PathBuf {
        self.root.path().join("checkout")
    }

    fn spec(&self) -> ExternalAgentLaunchSpec {
        spec_in(&self.checkout(), &self.root.path().join("members/m1"))
    }

    async fn start(&self) -> Box<dyn ExternalAgentProcess> {
        tokio::time::timeout(BOUND, self.launcher.start(self.spec()))
            .await
            .expect("start is bounded")
            .expect("the mock starts")
    }
}

fn spec_in(checkout: &Path, member_dir: &Path) -> ExternalAgentLaunchSpec {
    ExternalAgentLaunchSpec {
        model: "claude-haiku-4-5".into(),
        tools: vec!["Read".into(), "Edit".into(), "Bash".into()],
        mcp_config: json!({"mcpServers": {}}),
        settings: json!({"hooks": {}}),
        max_budget_usd: 1.5,
        checkout: checkout.to_path_buf(),
        member_dir: member_dir.to_path_buf(),
        credential: CredentialEnv {
            name: "CLAUDE_CODE_OAUTH_TOKEN".into(),
            value: "member-token".into(),
        },
    }
}

/// Close the input and wait for the exit, so the mock's record is whole.
async fn finish(process: &dyn ExternalAgentProcess) -> ExternalAgentExit {
    process.close_input().await;
    tokio::time::timeout(BOUND, process.exited())
        .await
        .expect("exit is bounded")
}

async fn events_to_turn_end(process: &dyn ExternalAgentProcess) -> Vec<ExternalAgentEvent> {
    let mut events = Vec::new();
    loop {
        let event = tokio::time::timeout(BOUND, process.next_event())
            .await
            .expect("an event is bounded")
            .expect("the stream reaches the turn's result");
        let end = matches!(event, ExternalAgentEvent::Result(_));
        events.push(event);
        if end {
            return events;
        }
    }
}

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
async fn a_user_turn_is_written_as_one_stream_json_user_message() {
    let rig = Rig::new("{\"type\": \"result\", \"subtype\": \"success\"}\n");
    let process = rig.start().await;
    process.send_user_turn("hello\nworld").await.unwrap();
    events_to_turn_end(process.as_ref()).await;
    finish(process.as_ref()).await;
    let input = rig.mock.recorded("input");
    assert_eq!(input.len(), 1, "one line per turn: {input:?}");
    let sent: serde_json::Value = serde_json::from_str(&input[0]).unwrap();
    assert_eq!(
        sent,
        json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "text", "text": "hello\nworld"}
        ]}})
    );
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

/// The mock's first recorded `kind` value, once its start record exists.
async fn recorded_at_start(mock: &MockClaude, kind: &str) -> String {
    tokio::time::timeout(BOUND, async {
        loop {
            if let Some(value) = mock.recorded(kind).into_iter().next() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the mock records {kind} at start"))
}

async fn until_retired(supervisor: &OwnedChildSupervisor, bound: Duration) {
    tokio::time::timeout(bound, async {
        while supervisor.slot_count() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the child is retired within the bound");
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
async fn a_cancelled_turn_never_leaves_half_a_line() {
    // The mock reads nothing for a while, so a turn larger than the pipe
    // is still being written when its send is cancelled.
    let rig = Rig::new("@stall-input 2\n");
    let process = rig.start().await;
    let big = "a".repeat(256 * 1024);
    let cancelled =
        tokio::time::timeout(Duration::from_millis(100), process.send_user_turn(&big)).await;
    assert!(
        cancelled.is_err(),
        "the send was still writing: {cancelled:?}"
    );
    process.send_user_turn("after").await.unwrap();
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    let input = rig.mock.recorded("input");
    let texts: Vec<String> = input
        .iter()
        .map(|line| {
            let sent: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
                panic!("a whole JSON line ({error}): {} bytes", line.len())
            });
            sent["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(texts.len(), 2, "each turn is its own whole line");
    assert_eq!(texts[0], big, "an accepted turn is written whole");
    assert_eq!(texts[1], "after");
}

#[tokio::test]
async fn closing_the_input_does_not_wait_behind_a_stuck_write() {
    // The mock never reads: a turn larger than the pipe never finishes.
    let rig = Rig::new("@stall-input 3600\n");
    let process = rig.start().await;
    let big = "a".repeat(256 * 1024);
    {
        let send = process.send_user_turn(&big);
        tokio::pin!(send);
        let pending = tokio::time::timeout(Duration::from_millis(200), &mut send).await;
        assert!(pending.is_err(), "the write is stuck: {pending:?}");
        tokio::time::timeout(Duration::from_secs(2), process.close_input())
            .await
            .expect("close_input returns while a write is stuck");
    }
    assert_eq!(
        process.send_user_turn("late").await,
        Err(crate::application::external_agent::dto::ExternalAgentInputError::Closed)
    );
    drop(process);
    until_retired(&rig.supervisor, FALLBACK_BOUND).await;
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
