//! #2390 review L1: the run watch and the UDS `swarm_control` port of one
//! process reach the same board, the process board, so a pushed `watch`
//! nudges the latch the watch waits on. A second board would make every
//! push a silent no-op.
use std::time::Duration;

use crate::application::swarm::ports::SwarmRunControl;
use crate::domain::swarm::watch::Nudge;
use crate::infrastructure::tools::swarm_bridge::{bind_process_board, process_board};

const CHILD: &str =
    "interface::tool_runtime::swarm_board_tests::the_process_contexts_share_the_process_board";
const CHILD_ENV: &str = "QUECTO_2390_SHARED_BOARD_CHILD";
const CHECKED: &str = "the process contexts share the process board";

/// Runs the check in a child test process under a container contract
/// (emulated, as `tool_runtime_profile_tests` does), so this process's
/// one-time process board is never bound here.
#[test]
fn the_watch_and_the_uds_control_share_the_process_board() {
    let checkout = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CHILD_ENV, "1")
        .env("QUECTO_SWARM_CHECKOUT", checkout.path())
        .env("QUECTO_SWARM_CONTAINER", "isolated-pid-v1")
        .env("QUECTO_SWARM_HOST_PID_NS", "pid:[0]")
        .env("QUECTO_SWARM_MEMBER", "coordinator")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(CHECKED),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The child: the contexts the watch (`join_current_process`) and the UDS
/// control port (`uds_multi`) are built from, both `swarm_context()`.
#[test]
fn the_process_contexts_share_the_process_board() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let board = bind_process_board(crate::composition::swarm::swarm_board());
    let watch = super::swarm_context().expect("a contracted swarm context");
    let control = super::swarm_context().expect("a contracted swarm context");
    assert!(watch.board.is_the_same_board(board));
    assert!(control.board.is_the_same_board(process_board().unwrap()));
    let latch = watch.board.watch_nudges(&watch.database());
    control.nudge_watch();
    assert_eq!(latch.wait(Duration::ZERO), Some(Nudge::Remote));
    println!("{CHECKED}");
}
