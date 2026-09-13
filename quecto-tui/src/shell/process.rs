//! Leader-only termination of a TUI-owned harness (#1956).
//!
//! After epic #1929 the harness owns its subagent tree: on SIGTERM it tears
//! the fleet down (bounded), persists, then exits, and every launcher-created
//! child is lifetime-bound to it. The TUI therefore signals **one** process —
//! the harness leader — with `kill(pid)` (never `kill(-pgid)`, never a
//! descendant), waits for that process to exit within a budget derived from
//! the harness's own teardown numbers, and SIGKILLs only that process after
//! the budget. A post-exit canary then reads `/proc` once and reports (never
//! signals) anything still naming the old leader as parent or group.

use std::time::Duration;

// ── Budget derivation ───────────────────────────────────────────────────
//
// The numbers below mirror the harness's documented teardown budgets
// (`quecto-agentic-harness`): `PROTOCOL_ACK_TIMEOUT` in
// `infrastructure/processes/direct_child_routing.rs`,
// `TerminationBudget::DEFAULT` in `infrastructure/processes/owned_child_supervisor.rs`,
// `COMPENSATION_WAIT_SLACK` / `DEFAULT_COMPENSATION_WAIT` in
// `infrastructure/tools/subagent_teardown_registry.rs`, and
// `FORCE_EXIT_AFTER` in `interface/cli/uds_shutdown.rs`. The TUI does not
// depend on the harness crate, so they are restated here and pinned against
// the harness sources by `budget_tests.rs`.

/// Harness: bound on a child's protocol shutdown ACK.
pub const HARNESS_PROTOCOL_ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// Harness: how long an acknowledged child gets to exit before the fallback.
pub const HARNESS_EXIT_AFTER_ACK: Duration = Duration::from_secs(10);
/// Harness: TERM grace of the owned-handle fallback.
pub const HARNESS_TERM_GRACE: Duration = Duration::from_secs(2);
/// Harness: KILL grace of the owned-handle fallback.
pub const HARNESS_KILL_GRACE: Duration = Duration::from_secs(2);
/// Harness: slack a compensation observer allows past the ladder.
pub const HARNESS_COMPENSATION_WAIT_SLACK: Duration = Duration::from_secs(6);
/// Harness: the full owned-handle ladder (ACK + exit + TERM + KILL = 19 s).
pub const HARNESS_OWNED_HANDLE_LADDER: Duration = HARNESS_PROTOCOL_ACK_TIMEOUT
    .saturating_add(HARNESS_EXIT_AFTER_ACK)
    .saturating_add(HARNESS_TERM_GRACE)
    .saturating_add(HARNESS_KILL_GRACE);
/// Harness: per-child worst case of the fleet teardown — the ladder plus the
/// compensation slack (`DEFAULT_COMPENSATION_WAIT` = 25 s). The fleet settles
/// children concurrently, so this is also the batch worst case.
pub const HARNESS_COMPENSATION_WAIT: Duration =
    HARNESS_OWNED_HANDLE_LADDER.saturating_add(HARNESS_COMPENSATION_WAIT_SLACK);
/// Harness: a repeated SIGTERM past this point forces the harness out
/// (`FORCE_EXIT_AFTER` = 45 s). The TUI's budget must never exceed it — the
/// harness's own escape hatch is the outer bound, not the TUI's.
pub const HARNESS_FORCE_EXIT_AFTER: Duration = Duration::from_secs(45);
/// Room, past the fleet's worst case, for the harness to persist its session
/// and exit after the fleet settled.
pub const LEADER_PERSIST_SLACK: Duration = Duration::from_secs(5);

/// How long the TUI waits for the harness leader to exit after SIGTERM
/// before SIGKILLing that one process: the fleet's per-child worst case
/// (25 s) plus persist-and-exit slack (5 s) = 30 s. Above the fleet worst
/// case so a child mid-turn is never cut off before the harness can settle
/// it, and below the harness's own 45 s force-exit.
pub const LEADER_EXIT_BUDGET: Duration =
    HARNESS_COMPENSATION_WAIT.saturating_add(LEADER_PERSIST_SLACK);

/// Show the "waiting for the agent to settle its subagents…" notice once the
/// exit has taken this long.
pub const SETTLING_NOTICE_AFTER: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq)]
pub enum QuectodError {
    Overflow(u32),
    Zero,
}
impl std::fmt::Display for QuectodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overflow(pid) => write!(f, "PID {pid} exceeds i32::MAX, cannot safely convert"),
            Self::Zero => write!(f, "PID 0 would signal the caller's own process group"),
        }
    }
}
impl std::error::Error for QuectodError {}
pub fn checked_pid(pid: u32) -> Result<i32, QuectodError> {
    if pid == 0 {
        return Err(QuectodError::Zero);
    }
    i32::try_from(pid).map_err(|_| QuectodError::Overflow(pid))
}

/// Send `signal` to exactly one process. A non-positive pid is refused
/// (`kill(0)` / `kill(-n)` would address a group), so this can never widen
/// into a group signal.
pub(crate) fn signal_leader(pid: i32, signal: libc::c_int) -> libc::c_int {
    if pid <= 0 {
        return -1;
    }
    // SAFETY: positive pid; the caller holds the unreaped child, so no recycled pid.
    unsafe { libc::kill(pid, signal) }
}

/// How the leader ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderEnd {
    /// The leader had already exited before termination was requested.
    AlreadyExited,
    /// The leader exited on its own after SIGTERM, inside the budget.
    ExitedAfterTerm,
    /// The leader outlived the budget and was SIGKILLed (that one pid only).
    Killed,
    /// The handle carried no pid; nothing was signalled.
    NoPid,
}

/// Outcome of one leader termination: how it ended, how long the wait took,
/// and what the post-exit canary found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderTermination {
    pub pid: Option<u32>,
    pub end: LeaderEnd,
    pub waited: Duration,
    /// Pids still naming the old leader as parent or process group after it
    /// exited. Reported, never signalled. Always empty under the lifetime
    /// binding — a non-empty list is the evidence that it was not upheld.
    pub strays: Vec<i32>,
}

/// SIGTERM the leader, await its exit within `budget`, SIGKILL it past the
/// budget, then run the canary. Only `child`'s own pid is ever signalled.
pub async fn terminate_leader(
    child: &mut tokio::process::Child,
    budget: Duration,
) -> LeaderTermination {
    let started = tokio::time::Instant::now();
    let Some(raw_pid) = child.id() else {
        return LeaderTermination {
            pid: None,
            end: LeaderEnd::NoPid,
            waited: Duration::ZERO,
            strays: Vec::new(),
        };
    };
    let pid = checked_pid(raw_pid).ok();
    let end = match child.try_wait() {
        Ok(Some(_)) => LeaderEnd::AlreadyExited,
        _ => {
            if let Some(pid) = pid {
                signal_leader(pid, libc::SIGTERM);
            }
            match tokio::time::timeout(budget, child.wait()).await {
                Ok(_) => LeaderEnd::ExitedAfterTerm,
                Err(_) => {
                    // Tokio signals the unreaped handle's own pid only.
                    let _ = child.start_kill();
                    // Bounded so the TUI can never hang on an unreapable
                    // (D-state) leader; an unreaped handle is reaped by tokio
                    // later.
                    let _ = tokio::time::timeout(HARNESS_KILL_GRACE, child.wait()).await;
                    LeaderEnd::Killed
                }
            }
        }
    };
    let strays = pid.map(stray_processes_of).unwrap_or_default();
    if !strays.is_empty() {
        tracing::warn!(
            leader = raw_pid,
            ?strays,
            "processes still name the exited harness as parent or group; not signalled (#1956 canary)"
        );
    }
    LeaderTermination {
        pid: Some(raw_pid),
        end,
        waited: started.elapsed(),
        strays,
    }
}

/// The leader was already observed exited (and reaped) before termination
/// was requested: nothing to signal, only the canary to run.
pub fn already_exited(pid: Option<u32>) -> LeaderTermination {
    let strays = pid
        .and_then(|pid| checked_pid(pid).ok())
        .map(stray_processes_of)
        .unwrap_or_default();
    LeaderTermination {
        pid,
        end: if pid.is_some() {
            LeaderEnd::AlreadyExited
        } else {
            LeaderEnd::NoPid
        },
        waited: Duration::ZERO,
        strays,
    }
}

/// Post-exit canary: one pass over `/proc` for processes whose parent or
/// process group is `leader`. Read-only — nothing is signalled.
pub fn stray_processes_of(leader: i32) -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut strays: Vec<i32> = dir
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| *pid != leader)
        .filter(|pid| names_leader(*pid, leader))
        .collect();
    strays.sort_unstable();
    strays
}

/// Whether `/proc/<pid>/stat` names `leader` as ppid (field 4) or pgrp
/// (field 5). A process that vanished mid-read is not a stray.
fn names_leader(pid: i32, leader: i32) -> bool {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    parse_parent_and_group(&text).is_some_and(|(parent, group)| parent == leader || group == leader)
}

/// `(ppid, pgrp)` from a `/proc/<pid>/stat` line; the comm field may contain
/// spaces and parentheses, so fields are taken after the last `)`.
pub(crate) fn parse_parent_and_group(stat: &str) -> Option<(i32, i32)> {
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    let mut fields = rest.split_whitespace().skip(1);
    let parent = fields.next()?.parse().ok()?;
    let group = fields.next()?.parse().ok()?;
    Some((parent, group))
}

#[cfg(test)]
#[path = "process_budget_tests.rs"]
mod budget_tests;
#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
