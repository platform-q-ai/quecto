//! Leader-only termination of a TUI-owned harness (#1956).
//!
//! After epic #1929 the harness owns its subagent tree: on SIGTERM it tears
//! the fleet down (bounded), persists, then exits, and every launcher-created
//! child is lifetime-bound to it. The TUI therefore signals **one** process —
//! the harness leader — with `kill(pid)` (never `kill(-pgid)`, never a
//! descendant), waits for that process to exit within a settle budget derived
//! from the harness's fleet-teardown numbers, sends a second SIGTERM to arm
//! the harness's own force-exit and waits that out, and SIGKILLs only that
//! process after both. A post-exit canary then reads `/proc` once and reports
//! (never signals) anything still naming the old leader as parent or group.

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
/// compensation slack (`DEFAULT_COMPENSATION_WAIT` = 25 s).
pub const HARNESS_COMPENSATION_WAIT: Duration =
    HARNESS_OWNED_HANDLE_LADDER.saturating_add(HARNESS_COMPENSATION_WAIT_SLACK);
/// Harness: direct children settled concurrently per batch
/// (`DEFAULT_SETTLEMENT_BOUND`): a fleet of `n` unresponsive children takes
/// `ceil(n / 8)` batches of the per-child worst case.
pub const HARNESS_SETTLEMENT_BOUND: usize = 8;
/// Harness: passes one fleet run makes at most (`MAX_PASSES`) — later passes
/// catch children registered while the first ran. The worst case when the
/// TUI does not know the child count.
pub const HARNESS_MAX_PASSES: u32 = 3;
/// Harness: a **repeated** SIGTERM/SIGINT past this point forces the harness
/// out (`FORCE_EXIT_AFTER` = 45 s). One signal never arms it — so the TUI
/// sends a second SIGTERM after the fleet budget and waits this long before
/// its own SIGKILL: the harness's escape hatch is given its chance first.
pub const HARNESS_FORCE_EXIT_AFTER: Duration = Duration::from_secs(45);
/// Room, past the fleet's worst case, for the harness to persist its session
/// and exit after the fleet settled.
pub const LEADER_PERSIST_SLACK: Duration = Duration::from_secs(5);

/// Show the "waiting for the agent to settle its subagents…" notice once the
/// exit has taken this long.
pub const SETTLING_NOTICE_AFTER: Duration = Duration::from_secs(1);

/// The two waits of one leader termination (#1956):
///
/// 1. after the first SIGTERM, `settle` — the fleet teardown's worst case for
///    the roster the TUI last saw (`ceil(children / 8)` batches × 25 s, at
///    least one batch, at most the 3-pass worst case) plus 5 s persist slack;
/// 2. after a **second** SIGTERM (which arms the harness's own 45 s
///    force-exit), `force` — the harness's `FORCE_EXIT_AFTER`;
///
/// and only then SIGKILL of that one pid. Never a group, never a descendant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaderBudget {
    pub settle: Duration,
    pub force: Duration,
}

impl LeaderBudget {
    /// Budget for a leader whose live direct-child count the TUI knows
    /// (`None`: unknown, assume the 3-pass worst case).
    pub fn for_children(children: Option<usize>) -> Self {
        Self {
            settle: fleet_settle_budget(children),
            force: HARNESS_FORCE_EXIT_AFTER,
        }
    }

    /// Worst case with an unknown roster: 3 × 25 s + 5 s, then 45 s.
    pub const WORST_CASE: Self = Self {
        settle: Duration::from_secs(
            HARNESS_COMPENSATION_WAIT.as_secs() * HARNESS_MAX_PASSES as u64
                + LEADER_PERSIST_SLACK.as_secs(),
        ),
        force: HARNESS_FORCE_EXIT_AFTER,
    };

    /// Longest a caller can be held: both waits plus the KILL grace.
    pub fn total(self) -> Duration {
        self.settle
            .saturating_add(self.force)
            .saturating_add(HARNESS_KILL_GRACE)
    }
}

/// Batches the fleet needs for `children` unresponsive direct children,
/// clamped to `1..=HARNESS_MAX_PASSES`; unknown counts assume the maximum.
pub fn fleet_batches(children: Option<usize>) -> u32 {
    let Some(children) = children else {
        return HARNESS_MAX_PASSES;
    };
    let batches = children.div_ceil(HARNESS_SETTLEMENT_BOUND).max(1);
    u32::try_from(batches)
        .unwrap_or(HARNESS_MAX_PASSES)
        .min(HARNESS_MAX_PASSES)
}

/// `fleet_batches(children)` × 25 s + 5 s persist slack.
pub fn fleet_settle_budget(children: Option<usize>) -> Duration {
    HARNESS_COMPENSATION_WAIT
        .saturating_mul(fleet_batches(children))
        .saturating_add(LEADER_PERSIST_SLACK)
}

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
    /// The leader exited on its own after the first SIGTERM, inside the
    /// fleet-derived settle budget.
    ExitedAfterTerm,
    /// The leader outlived the settle budget, and exited after the second
    /// SIGTERM inside the harness's own force-exit window.
    ExitedAfterRepeatedTerm,
    /// The leader outlived both waits and was SIGKILLed (that one pid only).
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

/// The exact process a watcher owns: its pid and the kernel start time
/// (`/proc/<pid>/stat` field 22) read when it was spawned. After the leader
/// is reaped its pid may be recycled; the canary runs only while no live
/// process carries the pid with a different start time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaderIdentity {
    pub pid: Option<u32>,
    pub start: Option<u64>,
}

impl LeaderIdentity {
    /// Capture the identity of a just-spawned (unreaped) child.
    pub fn capture(pid: Option<u32>) -> Self {
        let start = pid
            .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok())
            .and_then(|stat| parse_start_time(&stat));
        Self { pid, start }
    }

    fn checked(self) -> Option<i32> {
        self.pid.and_then(|pid| checked_pid(pid).ok())
    }

    /// Whether the pid now belongs to a different process (recycled), so
    /// ppid/pgrp matches against it would be false positives.
    fn recycled(self) -> bool {
        let Some(pid) = self.pid else { return false };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        parse_start_time(&stat) != self.start
    }

    /// The canary for this leader after it exited, or empty if recycled.
    fn canary(self) -> Vec<i32> {
        if self.recycled() {
            return Vec::new();
        }
        self.checked().map(stray_processes_of).unwrap_or_default()
    }
}

/// First SIGTERM, wait `budget.settle`; second SIGTERM (arming the harness's
/// own force-exit), wait `budget.force`; then SIGKILL of that one pid and a
/// bounded reap; then the canary. Only `child`'s own pid is ever signalled.
pub async fn terminate_leader(
    child: &mut tokio::process::Child,
    budget: LeaderBudget,
    identity: LeaderIdentity,
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
        _ => signal_and_await(child, pid, budget).await,
    };
    let strays = identity.canary();
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

async fn signal_and_await(
    child: &mut tokio::process::Child,
    pid: Option<i32>,
    budget: LeaderBudget,
) -> LeaderEnd {
    if let Some(pid) = pid {
        signal_leader(pid, libc::SIGTERM);
    }
    if tokio::time::timeout(budget.settle, child.wait())
        .await
        .is_ok()
    {
        return LeaderEnd::ExitedAfterTerm;
    }
    // A repeated signal is what arms the harness's own force-exit.
    if let Some(pid) = pid {
        signal_leader(pid, libc::SIGTERM);
    }
    if tokio::time::timeout(budget.force, child.wait())
        .await
        .is_ok()
    {
        return LeaderEnd::ExitedAfterRepeatedTerm;
    }
    // Tokio signals the unreaped handle's own pid only.
    let _ = child.start_kill();
    // Bounded so the TUI can never hang on an unreapable (D-state) leader;
    // an unreaped handle is reaped by tokio later.
    let _ = tokio::time::timeout(HARNESS_KILL_GRACE, child.wait()).await;
    LeaderEnd::Killed
}

/// The leader was already observed exited (and reaped) before termination
/// was requested: nothing to signal, only the canary to run.
pub fn already_exited(identity: LeaderIdentity) -> LeaderTermination {
    LeaderTermination {
        pid: identity.pid,
        end: if identity.pid.is_some() {
            LeaderEnd::AlreadyExited
        } else {
            LeaderEnd::NoPid
        },
        waited: Duration::ZERO,
        strays: identity.canary(),
    }
}

/// Kernel start time (`/proc/<pid>/stat` field 22) — stable for the life of
/// one process, different for any later holder of the same pid.
pub(crate) fn parse_start_time(stat: &str) -> Option<u64> {
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Post-exit canary: one pass over `/proc` for processes whose parent or
/// process group is `leader`. Read-only — nothing is signalled.
///
/// By design it sees only what still names the leader: a subagent the
/// harness launched shares its group (and is what the lifetime binding must
/// have ended), whereas swarm members and bash tool children spawned with
/// their own `process_group(0)` / `setsid` are reparented to init with their
/// own group on leader exit and are invisible here.
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
