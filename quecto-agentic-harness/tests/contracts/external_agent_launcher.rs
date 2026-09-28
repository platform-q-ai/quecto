//! Contract for [`ExternalAgentLauncher`] (#2286), proven on the production
//! `ClaudeCodeLauncher` against the mock `claude`: a missing program is the
//! clear error; a start gives the member its own private directories
//! before the child runs; two members never share them.
use std::os::unix::fs::PermissionsExt;

use quecto::application::external_agent::ports::ExternalAgentLauncher;

use crate::mock_claude::{BOUND, MockClaudeRig, fixture};

#[tokio::test]
async fn a_missing_binary_is_the_clear_error() {
    let rig = MockClaudeRig::without_claude();
    let outcome = tokio::time::timeout(BOUND, rig.launcher.start(rig.spec("m1")))
        .await
        .expect("start is bounded");
    let error = outcome.err().expect("no claude on PATH is refused");
    assert_eq!(
        error.to_string(),
        "claude CLI not found on PATH (required for claude-code members)"
    );
    assert_eq!(rig.supervisor.slot_count(), 0, "nothing was spawned");
}

#[tokio::test]
async fn each_member_gets_its_own_private_directories_before_its_child_runs() {
    let rig = MockClaudeRig::replaying(&fixture("rt"));
    let one = rig.start("one").await;
    let two = rig.start("two").await;
    for member in ["one", "two"] {
        for dir in ["home", "claude-config"] {
            let path = rig.member_dir(member).join(dir);
            let mode = std::fs::metadata(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "{}", path.display());
        }
    }
    for process in [one, two] {
        process.close_input().await;
        let exit = tokio::time::timeout(BOUND, process.exited())
            .await
            .expect("exit is bounded");
        assert!(exit.is_clean(), "{exit:?}");
    }
}
