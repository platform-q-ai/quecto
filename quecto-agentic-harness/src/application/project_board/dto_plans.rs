//! What a plan's use cases take (#2481 M4b): a new version proposed, and
//! an edit of a draft.
use crate::domain::project_board::entities::plan::PlanItem;
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
    pub items: Vec<PlanItem>,
    pub team: Option<Team>,
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
    pub items: Option<Vec<PlanItem>>,
    pub team: Option<Option<Team>>,
    pub expected_updated: Option<Timestamp>,
}

impl PlanEdit {
    /// Whether the edit gives a field to change (an allowlist of its
    /// content fields).
    pub fn sets_something(&self) -> bool {
        self.approach.is_some() || self.items.is_some() || self.team.is_some()
    }
}
