//! The brief a started task's coordinator is given (S4, #2479): how the
//! task's accepted plan maps onto the swarm's own inputs. The coordinator
//! creates the run and its task items from it; every bound the swarm
//! board would refuse partway through a run is judged here first, so a
//! plan the swarm cannot carry is refused before anything starts.
//!
//! - **goal:** "Board task `<slug>`, plan v`<n>`: `<title>`".
//! - **criteria:** the two [`RUN_CRITERIA`]; a plan's own criteria are its
//!   task items' acceptance, which the run's 16 KiB of criteria cannot hold.
//! - **member_limit:** the coordinator plus every worker role's members.
//! - **deadline_in_seconds:** the team's deadline less now, at least ten
//!   minutes and at most seven days (clamped, and said to be); a day when
//!   it names none.
//! - **items:** in dependency order, each titled `<slug>: <title>`, its
//!   acceptance its criteria (or its title alone), its dependencies by slug.
//! - **constraints:** the approach, trimmed, and cut to fit with a marker
//!   where it was; then where the plan is in full.
//!
//! A brief is only of a [`Started`] task, so of its accepted plan. The
//! team is read field by field (a new field does not compile until it is
//! judged here). The role named [`COORDINATOR_ROLE`] chooses the
//! coordinator's model and effort. Worker roles size the pool only: their models and efforts are
//! not applied yet (owner decision, 2026-10-13), and the brief lists them
//! as such.
use std::fmt;

use crate::domain::project_board::entities::plan::{MAX_PLAN_VERSION, PlanItem};
use crate::domain::project_board::entities::task::MAX_TITLE_CHARS;
use crate::domain::project_board::entities::team::{Budget, Role, RoleLimits, Team};
use crate::domain::project_board::services::starts::Started;
use crate::domain::project_board::value_objects::slug::{MAX_SLUG_BYTES, Slug};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::Effort;
use crate::domain::swarm::json_text;
use crate::domain::swarm::records::CriterionKind;
use crate::domain::swarm::validation::{
    CRITERIA_MAX_BYTES, DEADLINE_HORIZON_SECONDS, MEMBER_LIMIT_MAX, TEXT_MAX_BYTES,
    TITLE_MAX_BYTES, has_content,
};

/// The most bytes a brief takes, measured as [`SwarmBrief::bytes`]: what a
/// coordinator's task text carries inline.
pub const MAX_BRIEF_BYTES: usize = 64 * 1024;
/// The room the rendered brief's fixed wording takes (S4c's template).
pub const TEMPLATE_BYTES: usize = 4096;
/// The room the template's labels take around each task item.
pub const TEMPLATE_ITEM_BYTES: usize = 64;
/// The most members a swarm admits, its coordinator one of them.
pub const MAX_SWARM_MEMBERS: u32 = MEMBER_LIMIT_MAX as u32;
/// The furthest deadline a swarm takes: seven days.
pub const MAX_DEADLINE_SECONDS: u64 = DEADLINE_HORIZON_SECONDS as u64;
/// The nearest deadline a run is started with: ten minutes.
pub const MIN_DEADLINE_SECONDS: u64 = 10 * 60;
/// A run's deadline when the team names none: a day.
pub const DEFAULT_DEADLINE_SECONDS: u64 = 24 * 60 * 60;
/// The role whose model and effort are the coordinator's.
pub const COORDINATOR_ROLE: &str = "coordinator";
/// What ends an approach cut to fit.
pub const CUT_MARKER: &str = "… (cut here; the rest is in the plan on the board)";

// The swarm's bounds, read as whole numbers, are what they say.
const _: () = assert!(MEMBER_LIMIT_MAX == 25 && DEADLINE_HORIZON_SECONDS == 604_800.0);
const _: () = assert!(MIN_DEADLINE_SECONDS < DEFAULT_DEADLINE_SECONDS);
const _: () = assert!(DEFAULT_DEADLINE_SECONDS < MAX_DEADLINE_SECONDS);

/// A run criterion, as the swarm's `create` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCriterion {
    pub id: &'static str,
    pub kind: CriterionKind,
    pub description: &'static str,
}

/// Every started run's criteria.
pub const RUN_CRITERIA: [RunCriterion; 2] = [
    RunCriterion {
        id: "task_items",
        kind: CriterionKind::Review,
        description: "Every task item is completed and accepted against its acceptance criteria.",
    },
    RunCriterion {
        id: "delivery_checks",
        kind: CriterionKind::Review,
        description: "The integrated result passes the project's own checks, as its \
                      contributing guide or CI defines them.",
    },
];

/// One task item to create on the swarm board, exactly as given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefItem {
    pub id: Slug,
    /// `<slug>: <title>`: how a fold-back finds the plan's item again.
    pub title: String,
    pub acceptance: Vec<String>,
    pub dependencies: Vec<Slug>,
}

/// The coordinator's model and effort, from the [`COORDINATOR_ROLE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleChoice {
    pub model: String,
    pub effort: Effort,
}

/// A worker role whose model and effort are not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotApplied {
    pub role: Slug,
    pub model: String,
    pub effort: Effort,
}

/// What a started task's coordinator is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmBrief {
    pub task: Slug,
    pub plan_version: u32,
    pub goal: String,
    pub constraints: Vec<String>,
    /// Whether the approach was cut to fit the constraints.
    pub approach_cut: bool,
    pub criteria: Vec<RunCriterion>,
    /// `None`: the plan names no team, and the coordinator sizes its pool.
    pub member_limit: Option<u32>,
    pub deadline_in_seconds: u64,
    /// Whether the team's deadline lay past the swarm's furthest.
    pub deadline_clamped: bool,
    pub usage_budget: Option<u64>,
    /// In dependency order: each after every item it depends on.
    pub items: Vec<BriefItem>,
    /// `None`: no coordinator role, and the launcher's model is used.
    pub coordinator: Option<RoleChoice>,
    pub not_applied: Vec<NotApplied>,
}

/// What of a team spawn cannot carry yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamField {
    Image,
    Turns,
    /// A coordinator role of more than one member.
    CoordinatorMembers,
    /// No worker role beside the coordinator.
    Workers,
}

impl TeamField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Turns => "turns",
            Self::CoordinatorMembers => "members",
            Self::Workers => "workers",
        }
    }
}

/// Which of a swarm task item's bounds a plan's item is past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemLimit {
    /// The title with its `<slug>: ` prefix, in UTF-8 bytes.
    Title,
    /// The acceptance list, encoded as the swarm board encodes it.
    Criteria,
}

impl ItemLimit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Criteria => "criteria",
        }
    }
}

/// Why a plan cannot brief a swarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefRefusal {
    /// The coordinator and the workers together: more than a swarm holds.
    TeamTooLarge {
        members: u32,
    },
    TeamUnsupported {
        field: TeamField,
    },
    DeadlinePassed,
    /// Less than [`MIN_DEADLINE_SECONDS`] off.
    DeadlineTooSoon,
    NoTaskItems,
    /// A task item past one of the swarm's bounds, by `bytes`.
    ItemTooLarge {
        item: Slug,
        limit: ItemLimit,
        bytes: usize,
    },
    BriefTooLarge {
        bytes: usize,
    },
    /// The coordinator role names a model with no provider: spawn takes
    /// `provider/model`, and none is guessed (S4c).
    CoordinatorModel,
}

impl BriefRefusal {
    /// The refusal's code, as an answer and a record name it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TeamTooLarge { .. } => "team_too_large",
            Self::TeamUnsupported { .. } => "team_unsupported",
            Self::DeadlinePassed => "deadline_passed",
            Self::DeadlineTooSoon => "deadline_too_soon",
            Self::NoTaskItems => "no_task_items",
            Self::ItemTooLarge { .. } => "item_too_large",
            Self::BriefTooLarge { .. } => "brief_too_large",
            Self::CoordinatorModel => "coordinator_model",
        }
    }
}

impl fmt::Display for BriefRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TeamTooLarge { members } => write!(
                f,
                "the team needs {members} members with its coordinator; \
                 a swarm holds at most {MAX_SWARM_MEMBERS}"
            ),
            Self::TeamUnsupported { field } => f.write_str(match field {
                TeamField::Image => "a team's image is not supported yet: leave it out of the plan",
                TeamField::Turns => {
                    "a role's turn limit is not supported yet: leave it out of the plan"
                }
                TeamField::CoordinatorMembers => {
                    "a team's coordinator role must have exactly one member"
                }
                TeamField::Workers => {
                    "a team needs at least one worker role besides the coordinator"
                }
            }),
            Self::DeadlinePassed => {
                f.write_str("the team's deadline has passed: propose a plan with a later one")
            }
            Self::DeadlineTooSoon => f.write_str(
                "the team's deadline is too soon: \
                     propose a plan with a deadline at least 10 minutes off",
            ),
            Self::NoTaskItems => {
                f.write_str("the plan has no task items: add at least one before starting")
            }
            Self::ItemTooLarge {
                item,
                limit: ItemLimit::Title,
                bytes,
            } => write!(
                f,
                "task item {}'s title is {bytes} bytes with its prefix; \
                 a swarm takes at most {TITLE_MAX_BYTES}",
                item.as_str()
            ),
            Self::ItemTooLarge {
                item,
                limit: ItemLimit::Criteria,
                bytes,
            } => write!(
                f,
                "task item {}'s criteria are {bytes} bytes once encoded; \
                 a swarm takes at most {TEXT_MAX_BYTES}",
                item.as_str()
            ),
            Self::CoordinatorModel => f.write_str(
                "the coordinator role's model must name its provider, as provider/model: \
                 propose a plan that does",
            ),
            Self::BriefTooLarge { bytes } => write!(
                f,
                "the brief is {bytes} bytes and a coordinator takes at most \
                 {MAX_BRIEF_BYTES}: shorten the plan's task items"
            ),
        }
    }
}

impl std::error::Error for BriefRefusal {}

/// A plan that cannot brief a swarm, refused by its code and words (S4c).
impl From<BriefRefusal> for super::dto::ProjectBoardError {
    fn from(refusal: BriefRefusal) -> Self {
        Self::Brief {
            code: refusal.code(),
            reason: refusal.to_string(),
        }
    }
}

// The goal always fits: its words, a slug, a version (at most three
// digits) and a task title (at most four bytes a character).
const _: () = assert!(
    "Board task , plan v: ".len() + MAX_SLUG_BYTES + 3 + 4 * MAX_TITLE_CHARS <= TEXT_MAX_BYTES
);
const _: () = assert!(MAX_PLAN_VERSION <= 999);

impl SwarmBrief {
    /// The brief's size as a coordinator reads it, in UTF-8 bytes: every
    /// text and number it holds, a line each, and the template's room
    /// ([`TEMPLATE_BYTES`], and [`TEMPLATE_ITEM_BYTES`] an item).
    pub fn bytes(&self) -> usize {
        let line = |text: &str| text.len() + 1;
        let number = |value: u64| value.to_string().len() + 1;
        let criteria: usize = self
            .criteria
            .iter()
            .map(|c| line(c.id) + line(c.kind.as_str()) + line(c.description))
            .sum();
        let items: usize = self
            .items
            .iter()
            .map(|item| {
                TEMPLATE_ITEM_BYTES
                    + line(&item.title)
                    + item.acceptance.iter().map(|text| line(text)).sum::<usize>()
                    + item
                        .dependencies
                        .iter()
                        .map(|id| line(id.as_str()))
                        .sum::<usize>()
            })
            .sum();
        let coordinator: usize = self
            .coordinator
            .iter()
            .map(|choice| {
                line(COORDINATOR_ROLE) + line(&choice.model) + line(choice.effort.as_str())
            })
            .sum();
        let workers: usize = self
            .not_applied
            .iter()
            .map(|role| line(role.role.as_str()) + line(&role.model) + line(role.effort.as_str()))
            .sum();
        let numbers = number(u64::from(self.plan_version))
            + self
                .member_limit
                .map_or(0, |limit| number(u64::from(limit)))
            + number(self.deadline_in_seconds)
            + self.usage_budget.map_or(0, number);
        TEMPLATE_BYTES
            + line(self.task.as_str())
            + line(&self.goal)
            + self
                .constraints
                .iter()
                .map(|text| line(text))
                .sum::<usize>()
            + criteria
            + items
            + coordinator
            + workers
            + numbers
    }
}

/// The brief for `started`'s accepted plan, at `now`.
///
/// # Errors
/// A team spawn cannot carry (an image, a turn limit, a coordinator role of
/// more than one member, no worker role) or larger than a swarm; a deadline
/// passed or too soon; no task items, or one past the swarm's bounds; a
/// brief past [`MAX_BRIEF_BYTES`].
pub fn brief_of(started: &Started, now: &Timestamp) -> Result<SwarmBrief, BriefRefusal> {
    let (task, planned) = (started.task().fields(), started.plan().fields());
    debug_assert!(
        planned.task == task.id && task.accepted_plan == Some(planned.version),
        "a start runs the task's accepted plan"
    );
    let terms = match &planned.team {
        Some(team) => terms_of(team)?,
        None => Terms::default(),
    };
    let (deadline_in_seconds, deadline_clamped) = match terms.deadline {
        Some(deadline) => match u64::try_from(deadline.seconds_since(now)) {
            Ok(left) if left > MAX_DEADLINE_SECONDS => (MAX_DEADLINE_SECONDS, true),
            Ok(left) if left >= MIN_DEADLINE_SECONDS => (left, false),
            Ok(1..) => return Err(BriefRefusal::DeadlineTooSoon),
            Ok(0) | Err(_) => return Err(BriefRefusal::DeadlinePassed),
        },
        None => (DEFAULT_DEADLINE_SECONDS, false),
    };
    if planned.items.is_empty() {
        return Err(BriefRefusal::NoTaskItems);
    }
    let items = in_order(&planned.items)
        .into_iter()
        .map(brief_item)
        .collect::<Result<Vec<_>, _>>()?;
    let goal = format!(
        "Board task {}, plan v{}: {}",
        task.id.as_str(),
        planned.version,
        task.title
    );
    assert!(
        has_content(&goal) && goal.len() <= TEXT_MAX_BYTES,
        "the goal fits"
    );
    assert!(
        criteria_bytes(&RUN_CRITERIA) <= CRITERIA_MAX_BYTES,
        "the run's criteria fit"
    );
    let pointer = format!(
        "The full plan is on the project board (task {}, plan v{}); \
         ask your parent for any part cut here.",
        task.id.as_str(),
        planned.version
    );
    let (constraints, approach_cut) = constraints(&planned.approach, pointer);
    let brief = SwarmBrief {
        task: task.id.clone(),
        plan_version: planned.version,
        goal,
        constraints,
        approach_cut,
        criteria: RUN_CRITERIA.to_vec(),
        member_limit: terms.member_limit,
        deadline_in_seconds,
        deadline_clamped,
        usage_budget: terms.usage_budget,
        items,
        coordinator: terms.coordinator,
        not_applied: terms.not_applied,
    };
    match brief.bytes() {
        bytes if bytes <= MAX_BRIEF_BYTES => Ok(brief),
        bytes => Err(BriefRefusal::BriefTooLarge { bytes }),
    }
}

/// What a team makes of the run: its member limit, the coordinator's
/// choice, the worker roles not applied, its budget and its deadline.
#[derive(Default)]
struct Terms<'a> {
    member_limit: Option<u32>,
    coordinator: Option<RoleChoice>,
    not_applied: Vec<NotApplied>,
    usage_budget: Option<u64>,
    deadline: Option<&'a Timestamp>,
}

/// `team`'s terms, read field by field, when spawn can carry every field
/// and a swarm holds the team.
fn terms_of(team: &Team) -> Result<Terms<'_>, BriefRefusal> {
    let unsupported = |field| Err(BriefRefusal::TeamUnsupported { field });
    let Team {
        roles,
        budget,
        deadline,
        image,
    } = team;
    match image {
        None => {}
        Some(_) => return unsupported(TeamField::Image),
    }
    let mut coordinator = None;
    let mut not_applied = Vec::new();
    let mut members: u32 = 1;
    for role in roles {
        let Role {
            name,
            model,
            effort,
            limits: RoleLimits {
                members: size,
                turns,
            },
        } = role;
        match turns {
            None => {}
            Some(_) => return unsupported(TeamField::Turns),
        }
        match (name.as_str() == COORDINATOR_ROLE, *size) {
            (true, 1) => {
                // Spawn takes `provider/model`; no provider is guessed.
                match model.split_once('/') {
                    Some((provider, id)) if !provider.is_empty() && !id.is_empty() => {}
                    Some(_) | None => return Err(BriefRefusal::CoordinatorModel),
                }
                coordinator = Some(RoleChoice {
                    model: model.clone(),
                    effort: *effort,
                });
            }
            (true, _) => return unsupported(TeamField::CoordinatorMembers),
            (false, size) => {
                members += u32::from(size);
                not_applied.push(NotApplied {
                    role: name.clone(),
                    model: model.clone(),
                    effort: *effort,
                });
            }
        }
    }
    if not_applied.is_empty() {
        return unsupported(TeamField::Workers);
    }
    if members > MAX_SWARM_MEMBERS {
        return Err(BriefRefusal::TeamTooLarge { members });
    }
    Ok(Terms {
        member_limit: Some(members),
        coordinator,
        not_applied,
        usage_budget: budget.as_ref().map(|Budget { tokens }| *tokens),
        deadline: deadline.as_ref(),
    })
}

/// `items` in dependency order, stable: each pass takes, in the plan's
/// order, every item whose dependencies are all taken.
fn in_order(items: &[PlanItem]) -> Vec<&PlanItem> {
    let mut taken: Vec<&PlanItem> = Vec::with_capacity(items.len());
    while taken.len() < items.len() {
        let before = taken.len();
        for item in items {
            let placed = |id: &Slug| taken.iter().any(|done| done.id == *id);
            if !placed(&item.id) && item.depends_on.iter().all(placed) {
                taken.push(item);
            }
        }
        assert!(
            taken.len() > before,
            "a plan's task item dependencies never loop"
        );
    }
    taken
}

/// The title a swarm task item is created with for plan item `item`,
/// `<slug>: <title>`: the key a fold-back matches it back by.
pub fn item_title(item: &PlanItem) -> String {
    format!("{}: {}", item.id.as_str(), item.title)
}

/// `item` as a swarm task item, within the swarm's bounds.
fn brief_item(item: &PlanItem) -> Result<BriefItem, BriefRefusal> {
    let title = item_title(item);
    let acceptance = match item.criteria.as_slice() {
        [] => vec![item.title.clone()],
        criteria => criteria.to_vec(),
    };
    debug_assert!(
        acceptance.iter().all(|line| has_content(line)),
        "a plan's lines are visible"
    );
    let too_large = |limit, bytes| {
        Err(BriefRefusal::ItemTooLarge {
            item: item.id.clone(),
            limit,
            bytes,
        })
    };
    match title.len() {
        bytes if bytes <= TITLE_MAX_BYTES => {}
        bytes => return too_large(ItemLimit::Title, bytes),
    }
    match json_list_bytes(acceptance.iter().map(String::as_str)) {
        bytes if bytes <= TEXT_MAX_BYTES => {}
        bytes => return too_large(ItemLimit::Criteria, bytes),
    }
    Ok(BriefItem {
        id: item.id.clone(),
        title,
        acceptance,
        dependencies: item.depends_on.clone(),
    })
}

/// The run's constraints, with whether the approach was cut: the approach
/// trimmed, if it says anything, whole when it fits the swarm's 8 KiB and
/// otherwise as much as fits ended by [`CUT_MARKER`]; then `pointer`.
fn constraints(approach: &str, pointer: String) -> (Vec<String>, bool) {
    let approach = approach.trim();
    if !has_content(approach) {
        return (vec![pointer], false);
    }
    let whole = vec![approach.to_owned(), pointer.clone()];
    if json_list_bytes(whole.iter().map(String::as_str)) <= TEXT_MAX_BYTES {
        return (whole, false);
    }
    // Room for the kept characters: the list's brackets and comma, the
    // pointer, and the cut text's own quotes and marker.
    let marker = json_text(CUT_MARKER).len() - 2;
    let room = TEXT_MAX_BYTES - 2 - 1 - json_text(&pointer).len() - 2 - marker;
    let mut used = 0;
    let kept: String = approach
        .chars()
        .take_while(|c| {
            used += json_text(c.encode_utf8(&mut [0; 4])).len() - 2;
            used <= room
        })
        .collect();
    let constraints = vec![format!("{kept}{CUT_MARKER}"), pointer];
    assert!(
        json_list_bytes(constraints.iter().map(String::as_str)) <= TEXT_MAX_BYTES,
        "the constraints fit"
    );
    (constraints, true)
}

/// `criteria`'s bytes as the swarm board's JSON spells the list: each an
/// object of its `description`, `id` and `kind`.
fn criteria_bytes(criteria: &[RunCriterion]) -> usize {
    let field = |key: &str, value: &str| json_text(key).len() + 1 + json_text(value).len();
    let objects: usize = criteria
        .iter()
        .map(|criterion| {
            2 + field("description", criterion.description)
                + 1
                + field("id", criterion.id)
                + 1
                + field("kind", criterion.kind.as_str())
        })
        .sum();
    2 + objects + criteria.len().saturating_sub(1)
}

/// `texts`' bytes as a JSON list of strings in the swarm board's encoding
/// (no spaces between items).
fn json_list_bytes<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    let (count, bytes) = texts
        .into_iter()
        .fold((0_usize, 0_usize), |(count, bytes), text| {
            (count + 1, bytes + json_text(text).len())
        });
    2 + bytes + count.saturating_sub(1)
}

#[path = "swarm_brief_text.rs"]
mod text;
pub use text::render;

#[cfg(test)]
#[path = "swarm_brief_tests.rs"]
mod tests;
