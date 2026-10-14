//! A started run's keeper (S4d, #2479): what it sees of the run's swarm,
//! what it carries between looks, and what one look comes to.
//!
//! A keeper looks every [`KEEPER_PERIOD`], every [`PAUSED_PERIOD`] while
//! the swarm is paused (a close comes next, and a closed swarm's
//! environment is torn down soon after), while its session lasts:
//! - **The claim first.** The claim's expiry is kept from the board as last
//!   read; when less than [`RENEW_BELOW_SECONDS`] are left, the board is
//!   read again (a fetch) and the claim renewed. A run found ended there
//!   (by `end_run`, or as `lost` by the merge that completed its task), or
//!   a task gone, ends the keeping. A renewal refused as contended, by an
//!   unreachable store or by the clock is tried again; any other refusal
//!   (another holds the task, none does) stops the renewing, and the
//!   keeper keeps watching so it can still fold the run back.
//! - **The swarm next**, as the session's live board view reads it, its
//!   run bound at its first sighting (another run there later is the
//!   swarm gone). Ended, it is folded back once (the fold-back reads the
//!   board again). Paused, its proposed end is kept: when its environment
//!   is then gone (a close tears a swarm down), that kept view is folded
//!   back. A view the board's limits cut short is never folded back.
//! - **Giving up.** A swarm gone with nothing kept, or stalled (its board
//!   unreadable, or its coordinator no live member) for [`STALL_LIMIT`]
//!   in a row once its run was sighted, or for [`START_GRACE`] from the
//!   start before it ever was, is given up: nothing is written, the
//!   renewing stops so the claim lapses, and the run is left for
//!   `end_run`, which the session is told. A swarm paused for
//!   [`PAUSED_LIMIT`] stops the renewing too, and the session is told.
use std::time::Duration;

use super::dto::ProjectBoardError;
use super::fold_back::SwarmRunView;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// How often a keeper looks.
pub const KEEPER_PERIOD: Duration = Duration::from_secs(60);
/// How often a keeper looks while its swarm is paused.
pub const PAUSED_PERIOD: Duration = Duration::from_secs(5);
/// A claim with less than this left (45 minutes) is renewed.
pub const RENEW_BELOW_SECONDS: i64 = 45 * 60;
/// How long a sighted swarm may stall before its keeper gives up.
pub const STALL_LIMIT: Duration = Duration::from_secs(30 * 60);
/// How long a swarm has from the start to show its run (a container to
/// make, a coordinator to boot and create the run) before a keeper gives
/// up on it.
pub const START_GRACE: Duration = Duration::from_secs(60 * 60);
/// How long a swarm may stay paused, never closed, before its keeper
/// stops renewing the claim (6 hours).
pub const PAUSED_LIMIT: Duration = Duration::from_secs(6 * 60 * 60);

/// A started run to keep: the task, its run and the environment its swarm
/// runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptRun {
    pub task: Slug,
    pub run: u32,
    pub environment_ref: String,
}

/// What a keeper carries from one look to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    /// Whether the claim is still the keeper's to renew.
    pub renewing: bool,
    /// The claim's expiry, as the board last read showed it.
    pub claim_expires: Option<Timestamp>,
    /// Whether the run names its environment on the board.
    pub adopted: bool,
    /// The swarm's run, bound at its first sighting.
    pub run_id: Option<String>,
    /// When (the board clock's monotonic time) the keeping began.
    pub began: Option<Duration>,
    /// Since when the sighted swarm has been stalled, while it is.
    pub stalled_since: Option<Duration>,
    /// Since when the swarm has been paused, and the view its proposed end
    /// would fold back.
    pub paused_since: Option<Duration>,
    pub proposed: Option<SwarmRunView>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            renewing: true,
            claim_expires: None,
            adopted: false,
            run_id: None,
            began: None,
            stalled_since: None,
            paused_since: None,
            proposed: None,
        }
    }
}

/// A renewal a look made, with the seconds the claim had left before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renewal {
    Renewed {
        left_s: i64,
    },
    Refused {
        left_s: i64,
        error: ProjectBoardError,
    },
}

/// Why a keeper stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperEnd {
    /// It folded the run back.
    Folded,
    /// The run had ended already: folded back, ended by hand, or lost by
    /// the merge that completed its task.
    Ended,
    /// The task, or its run, is no longer on the board (archived).
    Gone,
    /// The fold-back was refused for good; the run is left for `end_run`.
    FoldBackRefused,
    /// The swarm ended in a view the board's limits cut short, which is
    /// never folded back; the run is left for `end_run`.
    Truncated,
    /// The swarm was given up (gone with nothing kept, or stalled); the
    /// run is left for `end_run`.
    SwarmLost,
    /// Its session ended.
    SessionEnded,
}

impl KeeperEnd {
    /// Whether the run is left open for `end_run` (an allowlist).
    pub fn leaves_run_open(&self) -> bool {
        matches!(
            self,
            Self::FoldBackRefused | Self::Truncated | Self::SwarmLost
        )
    }
}

/// What the session is told of a kept run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperNotice {
    /// The keeping ended with the run left open: end it with `end_run`.
    LeftOpen(KeeperEnd),
    /// The swarm stayed paused past [`PAUSED_LIMIT`]: the claim is no longer
    /// renewed; close the swarm, or end the run.
    PausedTooLong,
}

/// What one look came to: a renewal it made, whether the keeping ends,
/// what the session is told, and when to look next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub renewal: Option<Renewal>,
    pub end: Option<KeeperEnd>,
    pub notice: Option<KeeperNotice>,
    pub next: Duration,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            renewal: None,
            end: None,
            notice: None,
            next: KEEPER_PERIOD,
        }
    }
}

impl Look {
    /// The keeping ends with `end`; one that leaves the run open tells the
    /// session.
    pub fn ending(end: KeeperEnd) -> Self {
        let notice = end
            .leaves_run_open()
            .then(|| KeeperNotice::LeftOpen(end.clone()));
        Self {
            end: Some(end),
            notice,
            ..Self::default()
        }
    }
}

#[cfg(test)]
#[path = "keeper_tests.rs"]
pub(super) mod tests;
