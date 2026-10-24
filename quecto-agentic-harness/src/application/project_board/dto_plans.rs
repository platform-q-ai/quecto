//! What a plan's use cases take (#2481 M4b, M4c): a new version
//! proposed, an edit of a draft, and a draft accepted or discarded.
use crate::domain::project_board::entities::plan::{PlanEntry, PlanItem};
use crate::domain::project_board::entities::team::Team;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// A new draft version of `task`'s plan, saying all of what it says: its
/// approach, its task items and its team (`None`: the coordinator picks
/// the pool, as without a plan). Its number follows the task's last
/// version, and who proposed it is the store's committer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProposal {
    pub task: Slug,
    /// Markdown.
    pub approach: String,
    /// Markdown (#2705 T3); `None`, or empty once normalised, for none.
    pub handoff: Option<String>,
    pub items: Vec<PlanItem>,
    pub team: Option<Team>,
    /// How its approach enters, as its front end's policy resolved it.
    pub entry: PlanEntry,
}

/// An edit of draft `version` of `task`'s plan, as a patch: a field left
/// `None` keeps what the draft holds, byte for byte, and one given
/// replaces it whole (`team: Some(None)` takes the team away). So an edit
/// of the task items never resends the approach, which may be longer than
/// one answer shows. `expected_updated` is the draft's `updated` as the
/// caller read it, when it read one: a draft changed since is refused,
/// never edited over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEdit {
    pub task: Slug,
    pub version: u32,
    /// Markdown.
    pub approach: Option<String>,
    /// Markdown (#2705 T3): the whole handoff, or, empty once normalised,
    /// none (the plan's file goes back to schema 1).
    pub handoff: Option<String>,
    pub items: Option<Vec<PlanItem>>,
    pub team: Option<Option<Team>>,
    pub expected_updated: Option<Timestamp>,
    /// How a new approach enters, as its front end's policy resolved it.
    pub entry: PlanEntry,
}

impl PlanEdit {
    /// Whether the edit gives a field to change (an allowlist of its
    /// content fields).
    pub fn sets_something(&self) -> bool {
        self.approach.is_some()
            || self.handoff.is_some()
            || self.items.is_some()
            || self.team.is_some()
    }
}

/// Draft `version` of `task`'s plan accepted as the task's plan (#2481
/// M4c), as the caller read it: `expected_updated` is the draft's
/// `updated` then, so the content accepted is the content reviewed, and
/// `expected_accepted` the version the task named as accepted then
/// (`None`: none), so an acceptance never supersedes one its caller did
/// not see. Both are required: an acceptance is never of a board unread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanAcceptance {
    pub task: Slug,
    pub version: u32,
    pub expected_updated: Timestamp,
    pub expected_accepted: Option<u32>,
}

/// Draft `version` of `task`'s plan discarded (it is kept), pinned to the
/// draft as the caller read it when `expected_updated` is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDiscard {
    pub task: Slug,
    pub version: u32,
    pub expected_updated: Option<Timestamp>,
}
