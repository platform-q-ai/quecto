//! The rig of the claude process adapter's tests (#2286): a launcher over a
//! mock `claude` replaying one scenario, a checkout and a member directory
//! in one temporary directory. The real CLI never runs here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::ClaudeCodeLauncher;
use crate::application::external_agent::dto::{
    CredentialEnv, ExternalAgentExit, ExternalAgentLaunchSpec,
};
use crate::application::external_agent::ports::{ExternalAgentLauncher, ExternalAgentProcess};
use crate::domain::external_agent::stream::ExternalAgentEvent;
use crate::infrastructure::processes::owned_child_supervisor::{
    OwnedChildSupervisor, TerminationBudget,
};
use crate::infrastructure::test_support::mock_claude::{MockClaude, write_mock_claude};

pub const BOUND: Duration = Duration::from_secs(20);

/// A short fallback, so a child that ignores its input's close (and TERM)
/// is ended within [`FALLBACK_BOUND`] rather than the default's seconds.
pub const FAST: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_millis(200),
    term_grace: Duration::from_millis(300),
    kill_grace: Duration::from_secs(2),
};

/// Well inside [`TerminationBudget::DEFAULT`]'s 14 s, well outside `FAST`.
pub const FALLBACK_BOUND: Duration = Duration::from_secs(6);

/// A turn's worth of scenario: one `result` line.
pub const RESULT_LINE: &str = "{\"type\": \"result\", \"subtype\": \"success\"}\n";

/// The parent environment every rig passes: what the member must get, and
/// what it must not.
pub const PARENT: &[(&str, &str)] = &[
    ("LANG", "C.UTF-8"),
    ("CLAUDECODE", "1"),
    ("CLAUDE_CODE_SESSION_ID", "parent-session"),
    ("ANTHROPIC_BASE_URL", "http://leak.invalid"),
    ("GH_TOKEN", "leak"),
    ("QUECTO_BASE_DIR", "/leak"),
];

pub struct Rig {
    pub root: tempfile::TempDir,
    pub mock: MockClaude,
    pub supervisor: Arc<OwnedChildSupervisor>,
    pub launcher: ClaudeCodeLauncher,
}

impl Rig {
    pub fn new(scenario: &str) -> Self {
        Self::build(scenario.as_bytes(), &[], |launcher| launcher)
    }

    /// A rig whose parent environment also holds `extra`, whose launcher
    /// `configure` adjusts.
    pub fn build(
        scenario: &[u8],
        extra: &[(&str, &str)],
        configure: impl FnOnce(ClaudeCodeLauncher) -> ClaudeCodeLauncher,
    ) -> Self {
        let root = tempfile::tempdir().unwrap();
        let scenario_path = root.path().join("scenario.jsonl");
        std::fs::write(&scenario_path, scenario).unwrap();
        let mock = write_mock_claude(&root.path().join("bin"), &scenario_path);
        std::fs::create_dir(root.path().join("checkout")).unwrap();
        let supervisor = Arc::new(OwnedChildSupervisor::new());
        let path = format!("{}:/usr/bin:/bin", mock.bin_dir.display());
        let parent = [("PATH", path.as_str())]
            .iter()
            .chain(PARENT)
            .chain(extra)
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect();
        let launcher = configure(
            ClaudeCodeLauncher::new(Arc::clone(&supervisor), parent).with_termination_budget(FAST),
        );
        Self {
            root,
            mock,
            supervisor,
            launcher,
        }
    }

    pub fn checkout(&self) -> PathBuf {
        self.root.path().join("checkout")
    }

    pub fn spec(&self) -> ExternalAgentLaunchSpec {
        spec_in(&self.checkout(), &self.root.path().join("members/m1"))
    }

    pub async fn start(&self) -> Box<dyn ExternalAgentProcess> {
        tokio::time::timeout(BOUND, self.launcher.start(self.spec()))
            .await
            .expect("start is bounded")
            .expect("the mock starts")
    }
}

pub fn spec_in(checkout: &Path, member_dir: &Path) -> ExternalAgentLaunchSpec {
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
pub async fn finish(process: &dyn ExternalAgentProcess) -> ExternalAgentExit {
    process.close_input().await;
    tokio::time::timeout(BOUND, process.exited())
        .await
        .expect("exit is bounded")
}

pub async fn events_to_turn_end(process: &dyn ExternalAgentProcess) -> Vec<ExternalAgentEvent> {
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

/// The mock's first recorded `kind` value, once its start record exists.
pub async fn recorded_at_start(mock: &MockClaude, kind: &str) -> String {
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

pub async fn until_retired(supervisor: &OwnedChildSupervisor, bound: Duration) {
    tokio::time::timeout(bound, async {
        while supervisor.slot_count() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the child is retired within the bound");
}

/// The text of each user turn the mock read, each a whole JSON line.
pub fn turns_read(mock: &MockClaude) -> Vec<String> {
    mock.recorded("input")
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
        .collect()
}
