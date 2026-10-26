//! What a fold-back comes to (S4): its result, and what it saw, for its
//! `board_fold_back` record; and the ended swarm's board it folds back
//! from, as the run reports port answers it (#2481 S4c follow-up).
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::run_outcomes::StopKind;
use crate::domain::project_board::value_objects::code_ref::CommitId;
use crate::domain::project_board::value_objects::telemetry::{
    FoldBackRecord, ItemCounts, RunOutcomeKind,
};
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::domain::swarm::entities::records::{RunState, TaskState};
use crate::domain::swarm::value_objects::lifecycle::MemberExit;

/// What came of a fold-back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldedBack {
    /// The run as the board now holds it.
    Folded(Run),
    /// The run (by its number) had ended already: nothing was written.
    Ended(u32),
}

/// What a fold-back saw (S4), for its `board_fold_back` record: gathered
/// only for a recorded fold-back (one given none notes nothing), and only
/// ever kinds and counts. Each fact is `None` where the fold-back did not
/// get that far, or (a run already ended) had nothing to fold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldBackFacts {
    pub run_outcome: Option<RunOutcomeKind>,
    pub items: Option<ItemCounts>,
    pub unmatched: Option<u64>,
    pub extra: Option<u64>,
    pub evidence: Option<u64>,
    pub evidence_dropped: Option<u64>,
    pub cost_tokens: Option<u64>,
    /// The write's attempts, and how many lost the race.
    pub attempts: Option<u64>,
    pub lost: Option<u64>,
    /// How many times its fetches found the tracking ref locked by
    /// another writer in the same clone (#2663).
    pub lock_races: Option<u64>,
    /// The UTF-8 bytes of the folded run's text.
    pub bytes: Option<u64>,
    /// The commit it wrote (#2663).
    pub head: Option<CommitId>,
}

impl FoldBackFacts {
    /// Writes each fact there is into `record`, leaving every field it has
    /// none for as it was.
    pub fn fill(&self, record: &mut FoldBackRecord) {
        let FoldBackRecord {
            task: _,
            run: _,
            board: _,
            outcome: _,
            run_outcome,
            items,
            unmatched,
            extra,
            evidence,
            evidence_dropped,
            cost_tokens,
            attempts,
            races_lost,
            lock_races,
            bytes,
            head,
            duration_us: _,
        } = record;
        let facts = [
            (unmatched, self.unmatched),
            (extra, self.extra),
            (evidence, self.evidence),
            (evidence_dropped, self.evidence_dropped),
            (cost_tokens, self.cost_tokens),
            (attempts, self.attempts),
            (races_lost, self.lost),
            (lock_races, self.lock_races),
            (bytes, self.bytes),
        ];
        for (field, fact) in facts {
            *field = fact.or(*field);
        }
        *run_outcome = self.run_outcome.or(*run_outcome);
        *items = self.items.or(*items);
        *head = self.head.clone().or(head.take());
    }
}

/// How a swarm ended: one of the statuses that end it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SwarmEnd {
    Succeeded,
    Failed,
    BudgetExhausted,
    Blocked,
    Cancelled,
}

impl SwarmEnd {
    pub const ALL: [Self; 5] = [
        Self::Succeeded,
        Self::Failed,
        Self::BudgetExhausted,
        Self::Blocked,
        Self::Cancelled,
    ];

    /// The end `status` is, when it ends the swarm (an allowlist: a
    /// swarm setting up, running or paused has not ended, nor has one
    /// whose status is any other text).
    pub fn of(status: &RunState) -> Option<Self> {
        match status.as_str() {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "budget-exhausted" => Some(Self::BudgetExhausted),
            "blocked" => Some(Self::Blocked),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// The board's outcome of a run that ended so.
    pub fn outcome(self) -> RunOutcome {
        match self {
            Self::Succeeded => RunOutcome::Succeeded,
            Self::Failed | Self::BudgetExhausted => RunOutcome::Failed,
            Self::Blocked | Self::Cancelled => RunOutcome::Stopped,
        }
    }

    /// How a run that ended so stopped, when it stopped: blocked, or
    /// cancelled (any other end is no stop, and moves its task by its
    /// outcome alone, #2705 T5).
    pub fn stop(self) -> StopKind {
        match self {
            Self::Blocked => StopKind::Blocked,
            Self::Succeeded | Self::Failed | Self::BudgetExhausted | Self::Cancelled => {
                StopKind::Cancelled
            }
        }
    }
}

/// Where a swarm item stood: one of the five statuses the swarm writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmItemState {
    Ready,
    Claimed,
    Submitted,
    Completed,
    Blocked,
}

impl SwarmItemState {
    /// The state `state` is, when the swarm writes it (an allowlist).
    pub fn of(state: &TaskState) -> Option<Self> {
        match state.as_str() {
            "ready" => Some(Self::Ready),
            "claimed" => Some(Self::Claimed),
            "submitted" => Some(Self::Submitted),
            "completed" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    /// How far an item standing so got: completed, submitted, claimed,
    /// ready, then blocked (an item blocked and created again is taken by
    /// its new copy).
    pub fn progress(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Ready => 1,
            Self::Claimed => 2,
            Self::Submitted => 3,
            Self::Completed => 4,
        }
    }

    /// Where an item read in this state with `blocker` most likely stood
    /// before a member's confirmed death blocked it (#2685), when one did:
    /// blocked, by exactly the text such a death writes
    /// ([`MemberExit::of_blocker`]). A board is kept only once its
    /// coordinator's teardown has ended its workers, so such a blocker is
    /// read as the teardown's rather than the item's own (a worker that
    /// died mid-run and was never recovered reads the same). It stood
    /// submitted when the board holds evidence for it (`evidence`: its
    /// stored evidence is not empty; a submission writes it, and a
    /// submitted item is never blocked by its worker), else claimed (its
    /// worker held it). `None`: not blocked by a death.
    pub fn before_death(self, blocker: Option<&str>, evidence: bool) -> Option<Self> {
        let by_death = blocker.and_then(MemberExit::of_blocker).is_some();
        match (self, by_death, evidence) {
            (Self::Blocked, true, true) => Some(Self::Submitted),
            (Self::Blocked, true, false) => Some(Self::Claimed),
            (Self::Blocked, false, _)
            | (Self::Ready | Self::Claimed | Self::Submitted | Self::Completed, _, _) => None,
        }
    }

    /// The board's state of a task item standing so.
    pub fn item_state(self) -> ItemState {
        match self {
            Self::Ready => ItemState::Todo,
            Self::Claimed => ItemState::Doing,
            Self::Submitted => ItemState::Review,
            Self::Completed => ItemState::Done,
            Self::Blocked => ItemState::Blocked,
        }
    }
}

/// One evidence entry a swarm item was submitted with.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwarmEvidence {
    pub artifact: String,
    pub revision: String,
}

/// One item of an ended swarm's board.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwarmItemView {
    pub title: String,
    pub state: SwarmItemState,
    /// The member holding it, by id.
    pub owner: Option<String>,
    pub evidence: Vec<SwarmEvidence>,
    /// Where it most likely stood before a member's confirmed death
    /// blocked it ([`SwarmItemState::before_death`]); `None`: no death did.
    pub stood_before_death: Option<SwarmItemState>,
}

impl SwarmItemView {
    /// Where this item stands for the fold-back (#2685): as its state
    /// reads, or, for one a confirmed death blocked, where it most likely
    /// stood before. Not proof: a submission released and claimed again
    /// keeps its evidence (the swarm board's release does not clear it),
    /// so it reads as submitted; and a blocker of the item's own that the
    /// death overwrote reads as claimed.
    pub fn standing(&self) -> SwarmItemState {
        debug_assert!(
            self.stood_before_death.is_none() || self.state == SwarmItemState::Blocked,
            "only a blocked item is blocked by a death"
        );
        self.stood_before_death.unwrap_or(self.state)
    }

    /// The board's state of the task item this swarm item stands for.
    pub fn item_state(&self) -> ItemState {
        self.standing().item_state()
    }
}

/// An ended swarm's board, as its live view (`get_swarm_board`) reads
/// it: how it ended, its items in the view's order, and the tokens it
/// was observed to spend.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwarmRunView {
    pub end: SwarmEnd,
    pub items: Vec<SwarmItemView>,
    pub observed_tokens: u64,
}
