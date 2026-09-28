//! The contract rig for the external-agent process ports (#2286): the
//! production [`ClaudeCodeLauncher`] against the mock `claude` written by
//! `quecto::infrastructure::test_support::mock_claude`, replaying a
//! stream-json scenario (a fixture under `tests/fixtures/claude_code/` or
//! one written by the test). The real CLI never runs here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use quecto::application::external_agent::dto::{CredentialEnv, ExternalAgentLaunchSpec};
use quecto::application::external_agent::ports::{ExternalAgentLauncher, ExternalAgentProcess};
use quecto::domain::external_agent::stream::ExternalAgentEvent;
use quecto::infrastructure::external_agents::claude_code::process::ClaudeCodeLauncher;
use quecto::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor;
use quecto::infrastructure::test_support::mock_claude::write_mock_claude;

/// The bound on every wait of a contract: a hang is a failure, not a stall.
pub const BOUND: Duration = Duration::from_secs(20);

/// A captured stream-json fixture.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude_code")
        .join(format!("{name}.stream.jsonl"))
}

/// A launcher, a mock replaying one scenario, a checkout and a base for
/// member directories, all in one temporary directory.
pub struct MockClaudeRig {
    pub root: tempfile::TempDir,
    pub supervisor: Arc<OwnedChildSupervisor>,
    pub launcher: ClaudeCodeLauncher,
}

impl MockClaudeRig {
    /// A rig whose `PATH` holds the mock replaying `scenario`.
    pub fn replaying(scenario: &Path) -> Self {
        let root = tempfile::tempdir().expect("rig tempdir");
        let mock = write_mock_claude(&root.path().join("bin"), scenario);
        let path = format!("{}:/usr/bin:/bin", mock.bin_dir.display());
        Self::build(root, path)
    }

    /// A rig whose `PATH` holds no `claude` at all.
    pub fn without_claude() -> Self {
        let root = tempfile::tempdir().expect("rig tempdir");
        let empty = root.path().join("empty-bin");
        std::fs::create_dir(&empty).expect("empty bin dir");
        let path = empty.display().to_string();
        Self::build(root, path)
    }

    fn build(root: tempfile::TempDir, path: String) -> Self {
        std::fs::create_dir(root.path().join("checkout")).expect("checkout");
        let supervisor = Arc::new(OwnedChildSupervisor::new());
        let parent = vec![
            (OsString::from("PATH"), OsString::from(path)),
            (OsString::from("CLAUDECODE"), OsString::from("1")),
        ];
        let launcher = ClaudeCodeLauncher::new(Arc::clone(&supervisor), parent);
        Self {
            root,
            supervisor,
            launcher,
        }
    }

    /// The member directory of `member`.
    pub fn member_dir(&self, member: &str) -> PathBuf {
        self.root.path().join("members").join(member)
    }

    /// A launch spec for `member`.
    pub fn spec(&self, member: &str) -> ExternalAgentLaunchSpec {
        ExternalAgentLaunchSpec {
            model: "claude-haiku-4-5".into(),
            tools: vec!["Read".into(), "Bash".into()],
            mcp_config: serde_json::json!({"mcpServers": {}}),
            settings: serde_json::json!({}),
            max_budget_usd: 1.0,
            checkout: self.root.path().join("checkout"),
            member_dir: self.member_dir(member),
            credential: CredentialEnv {
                name: "ANTHROPIC_API_KEY".into(),
                value: "mock-key".into(),
            },
        }
    }

    /// Start `member`'s process, bounded.
    pub async fn start(&self, member: &str) -> Box<dyn ExternalAgentProcess> {
        tokio::time::timeout(BOUND, self.launcher.start(self.spec(member)))
            .await
            .expect("start is bounded")
            .expect("the mock starts")
    }
}

/// The events up to and including the turn's `result`, bounded.
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
