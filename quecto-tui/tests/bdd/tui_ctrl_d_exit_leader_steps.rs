//! #1956 steps for `tui_ctrl_d_exit.feature`: leader-only termination of a
//! REAL stand-in harness through the production ordinary-exit path.
//!
//! The stand-in is a shell script spawned in its own process group under the
//! production child watcher. It backgrounds a child that logs every signal it
//! receives (so "the TUI never signalled the child" is evidence, not
//! absence), then behaves per scenario on SIGTERM: settle the child and exit,
//! settle slowly, ignore TERM, or exit leaving the child behind.

use super::with_harness;
use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;

/// A real stand-in harness adopted by the TUI for one scenario.
#[derive(Debug)]
pub struct ExitFixture {
    /// Files the stand-in writes: `child.pid`, `child.signals` (every signal
    /// the child received), `order` (`child-gone` before the leader exits)
    /// and `term-handled` (the leader's TERM handler ran to completion).
    pub dir: tempfile::TempDir,
    pub harness_pid: u32,
    pub child_pid: i32,
    /// Post-cleanup finalization messages, or the watcher's outcome name.
    pub report: Vec<String>,
}

impl ExitFixture {
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }
    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.path(name))
            .ok()
            .map(|s| s.trim().to_string())
    }
}

impl Drop for ExitFixture {
    fn drop(&mut self) {
        // Scenario cleanup only (detach and canary scenarios leave processes
        // alive on purpose); the production path never does this.
        for pid in [self.child_pid, self.harness_pid as i32] {
            // SAFETY: pids this scenario spawned and still owns.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes liveness of a pid this scenario spawned.
    let probed = unsafe { libc::kill(pid, 0) == 0 };
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.get(s.rfind(')')? + 2..)?.chars().next());
    probed && state.is_some_and(|st| st != 'Z' && st != 'X')
}

/// Spawn the stand-in: a signal-logging child, then `leader` (with `{dir}`
/// substituted) as the leader's own behaviour.
fn adopt(world: &mut TuiWorld, leader: &str) {
    let dir = tempfile::tempdir().expect("fixture dir");
    let d = dir.path().display().to_string();
    let child = format!(
        "sh -c 'trap \"echo TERM >> {d}/child.signals\" TERM; \
         trap \"echo INT >> {d}/child.signals\" INT; \
         trap \"echo HUP >> {d}/child.signals\" HUP; \
         echo $$ > {d}/child.pid; while :; do sleep 0.05; done' & kid=$!; "
    );
    let script = format!("{child}{}", leader.replace("{dir}", &d));
    let harness_pid = with_harness(world, |h| h.adopt_owned_harness_script(&script));
    let child_pid_file = dir.path().join("child.pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let child_pid = loop {
        if let Some(pid) = std::fs::read_to_string(&child_pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "stand-in child pid not written"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    // Let the leader install its trap before any signal arrives.
    std::thread::sleep(std::time::Duration::from_millis(100));
    world.tui_exit = Some(ExitFixture {
        dir,
        harness_pid,
        child_pid,
        report: Vec::new(),
    });
}

fn fixture(world: &TuiWorld) -> &ExitFixture {
    world.tui_exit.as_ref().expect("exit fixture")
}

// ── Given ──────────────────────────────────────────────────────────────────

#[given("the TUI owns a stand-in harness that settles its own child on SIGTERM")]
fn owns_settling_harness(world: &mut TuiWorld) {
    // On TERM: end the child (the harness's own authority), let the child's
    // orphaned 50 ms `sleep` run out so the fixture leaves nothing transient
    // in the group, record the order, mark the TERM handled, exit 0.
    adopt(
        world,
        "trap 'kill -KILL $kid; wait $kid; sleep 0.3; echo child-gone > {dir}/order; \
         : > {dir}/term-handled; exit 0' TERM; while :; do sleep 0.05; done",
    );
}

#[given("the TUI owns a stand-in harness that exits 2 seconds after SIGTERM")]
fn owns_slow_harness(world: &mut TuiWorld) {
    adopt(
        world,
        "trap 'kill -KILL $kid; wait $kid; sleep 2; : > {dir}/term-handled; exit 0' TERM; \
         while :; do sleep 0.05; done",
    );
}

#[given("the TUI owns a stand-in harness that ignores SIGTERM")]
fn owns_term_ignoring_harness(world: &mut TuiWorld) {
    adopt(world, "trap '' TERM; while :; do sleep 0.05; done");
}

#[given("the TUI owns a stand-in harness whose child outlives it")]
fn owns_harness_with_outliving_child(world: &mut TuiWorld) {
    // The leader exits on TERM without ending its child: the child stays in
    // the leader's process group and is exactly what the canary must name.
    adopt(
        world,
        "trap ': > {dir}/term-handled; exit 0' TERM; while :; do sleep 0.05; done",
    );
}

#[given(
    regex = r"^the leader budget is (\d+) milliseconds to settle and (\d+) milliseconds after a repeated SIGTERM$"
)]
fn leader_budget(world: &mut TuiWorld, settle_ms: u64, force_ms: u64) {
    with_harness(world, |h| {
        h.set_leader_budget(
            std::time::Duration::from_millis(settle_ms),
            std::time::Duration::from_millis(force_ms),
        );
    });
}

#[given("the TUI owns a stand-in harness that exits only on a repeated SIGTERM")]
fn owns_repeated_term_harness(world: &mut TuiWorld) {
    // First TERM: re-arm the trap; second TERM: settle the child and exit —
    // the shape of the harness's own force-exit.
    adopt(
        world,
        "trap 'trap \"kill -KILL $kid; wait $kid; sleep 0.3; : > {dir}/second-term; exit 0\" TERM' TERM; \
         while :; do sleep 0.05; done",
    );
}

#[given("the exit policy is detach-on-exit")]
fn detach_on_exit(world: &mut TuiWorld) {
    with_harness(world, |h| h.set_kill_owned_on_exit(false));
}

// ── When ───────────────────────────────────────────────────────────────────

#[when("the TUI finalizes ordinary exit")]
fn finalize_ordinary_exit(world: &mut TuiWorld) {
    let rt = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("TUI harness").0;
    let report = rt.block_on(h.finalize_exit());
    world.tui_exit.as_mut().expect("exit fixture").report = report;
}

#[when(regex = r"^the owned harness is terminated through the watcher with a (\d+) second budget$")]
fn terminate_through_watcher(world: &mut TuiWorld, secs: u64) {
    let rt = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("TUI harness").0;
    let outcome = rt
        .block_on(h.terminate_adopted_harness(std::time::Duration::from_secs(secs)))
        .expect("watcher reports");
    let end = TuiHarness::leader_end_name(outcome.end).to_string();
    world.tui_exit.as_mut().expect("exit fixture").report = vec![end];
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then("the harness child should be gone before the harness exited")]
fn child_gone_before_harness_exited(world: &mut TuiWorld) {
    let f = fixture(world);
    assert_eq!(
        f.read("order").as_deref(),
        Some("child-gone"),
        "the harness recorded its child's end before exiting"
    );
    assert!(!process_alive(f.child_pid), "child must be gone");
    assert!(!process_alive(f.harness_pid as i32), "harness must be gone");
}

#[then("the harness should have exited after SIGTERM without SIGKILL")]
fn harness_exited_after_term(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(
        f.path("term-handled").exists(),
        "the harness's TERM handler ran to completion, so no SIGKILL cut it off"
    );
    assert!(!process_alive(f.harness_pid as i32), "harness must be gone");
}

#[then("the harness child should have received no signal from the TUI")]
fn child_received_no_signal(world: &mut TuiWorld) {
    let f = fixture(world);
    assert_eq!(
        f.read("child.signals"),
        None,
        "the TUI must never signal a descendant"
    );
}

#[then("the exit report should mention no SIGKILL and no stray")]
fn report_clean(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(
        !f.report
            .iter()
            .any(|m| m.contains("SIGKILL") || m.contains("canary")),
        "clean exit reports nothing about the leader: {:?}",
        f.report
    );
}

#[then("the settling notice should have been shown with elapsed seconds")]
fn settling_notice_shown(world: &mut TuiWorld) {
    let shown = with_harness(world, |h| h.settling_notices_shown());
    assert!(
        shown
            .first()
            .is_some_and(|m| m == "waiting for the agent to settle its subagents… (1s)"),
        "notice appears after ~1 s with the elapsed time: {shown:?}"
    );
    assert!(
        shown.len() >= 2 && shown[1].ends_with("(2s)"),
        "notice is updated each second: {shown:?}"
    );
}

#[then("the settling notice should be dismissed")]
fn settling_notice_dismissed(world: &mut TuiWorld) {
    assert!(
        !with_harness(world, |h| h.settling_notice_visible()),
        "the notice is dismissed once the leaders are gone"
    );
}

#[then("the harness should have been SIGKILLed after the budget")]
fn harness_killed_after_budget(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(
        !f.path("term-handled").exists(),
        "a TERM-ignoring harness has no handled marker"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while process_alive(f.harness_pid as i32) {
        assert!(
            std::time::Instant::now() < deadline,
            "the leader must really be dead after SIGKILL"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[then("the harness should have exited on the repeated SIGTERM without SIGKILL")]
fn harness_exited_on_repeated_term(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(
        f.path("second-term").exists(),
        "the second TERM's handler ran to completion"
    );
    assert!(!process_alive(f.harness_pid as i32));
    assert!(
        !f.report.iter().any(|m| m.contains("SIGKILL")),
        "no SIGKILL: {:?}",
        f.report
    );
}

#[then("the exit report should name the SIGKILLed harness pid")]
fn report_names_killed_pid(world: &mut TuiWorld) {
    let f = fixture(world);
    let expected = format!(
        "harness pid {} did not exit within 300ms of SIGTERM nor 200ms of a repeated SIGTERM; sent SIGKILL to that process only",
        f.harness_pid
    );
    assert!(
        f.report.iter().any(|m| m.contains(&expected)),
        "report names the killed leader: {:?}",
        f.report
    );
}

#[then("the exit report should name the stray pid as not signalled")]
fn report_names_stray(world: &mut TuiWorld) {
    let f = fixture(world);
    // The stray's own `sleep` grandchild shares the group and may be named
    // too; the stray itself must be, and nothing was signalled.
    let line = f
        .report
        .iter()
        .find(|m| m.starts_with("post-exit canary: pids ["))
        .unwrap_or_else(|| panic!("canary line missing: {:?}", f.report));
    let (pids, rest) = line["post-exit canary: pids [".len()..]
        .split_once(']')
        .expect("pid list");
    assert!(
        pids.split(", ").any(|p| p == f.child_pid.to_string()),
        "canary names the stray {}: {line}",
        f.child_pid
    );
    assert_eq!(
        rest,
        format!(
            " still name harness pid {} as parent or process group; not signalled",
            f.harness_pid
        )
    );
}

#[then("the stray process should still be alive and unsignalled")]
fn stray_alive_unsignalled(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(
        process_alive(f.child_pid),
        "the canary reports, it never signals"
    );
    assert_eq!(f.read("child.signals"), None, "stray received no signal");
}

#[then("the harness should still be running")]
fn harness_still_running(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(
        process_alive(f.harness_pid as i32),
        "detach-on-exit leaves the harness alone"
    );
    assert_eq!(
        with_harness(world, |h| h.owned_watches_remaining()),
        1,
        "detach leaves the owned watch for drop"
    );
}

#[then("the harness child should still be running")]
fn harness_child_still_running(world: &mut TuiWorld) {
    let f = fixture(world);
    assert!(process_alive(f.child_pid));
    assert_eq!(f.read("child.signals"), None);
}

#[then("the watcher should report the harness exited after SIGTERM")]
fn watcher_reports_exited_after_term(world: &mut TuiWorld) {
    assert_eq!(fixture(world).report, vec!["exited-after-term".to_string()]);
}
