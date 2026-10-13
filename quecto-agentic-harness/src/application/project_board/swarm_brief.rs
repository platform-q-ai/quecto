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
//! - **deadline_in_seconds:** the team's deadline less now, at most seven
//!   days; a day when it names none.
//! - **items:** in dependency order, each titled `<slug>: <title>`, its
//!   acceptance its criteria (or its title alone), its dependencies by slug.
//! - **constraints:** the approach, cut to fit, and where to read the plan
//!   in full.
//!
//! The role named [`COORDINATOR_ROLE`] chooses the coordinator's model and
//! effort. Worker roles size the pool only: their models and efforts are
//! not applied yet (owner decision, 2026-10-13), and the brief lists them
//! as such.
use std::fmt;

use crate::domain::project_board::entities::plan::{MAX_PLAN_VERSION, Plan, PlanItem};
use crate::domain::project_board::entities::task::{MAX_TITLE_CHARS, Task};
use crate::domain::project_board::entities::team::Team;
use crate::domain::project_board::value_objects::slug::{MAX_SLUG_BYTES, Slug};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::Effort;
use crate::domain::swarm::policy::MAX_EXTENSION_SECONDS;
use crate::domain::swarm::records::CriterionKind;
use crate::domain::swarm::validation::{CRITERIA_MAX_BYTES, TEXT_MAX_BYTES, has_content};

/// The most bytes a brief takes, measured as [`SwarmBrief::bytes`]: what a
/// coordinator's task text carries inline.
pub const MAX_BRIEF_BYTES: usize = 64 * 1024;
/// The most members a swarm admits, its coordinator one of them.
pub const MAX_SWARM_MEMBERS: u32 = 25;
/// The furthest deadline a swarm takes: seven days.
pub const MAX_DEADLINE_SECONDS: u64 = MAX_EXTENSION_SECONDS.unsigned_abs();
/// A run's deadline when the team names none: a day.
pub const DEFAULT_DEADLINE_SECONDS: u64 = 24 * 60 * 60;
/// The most bytes a swarm task item's title takes.
pub const MAX_ITEM_TITLE_BYTES: usize = 1024;
/// The role whose model and effort are the coordinator's.
pub const COORDINATOR_ROLE: &str = "coordinator";

/// A run criterion, as the swarm's `create` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCriterion {
    pub id: &'static str,
    pub kind: CriterionKind,
    pub description: &'static str,
}

/// Every started run's criteria (wording pending the owner's approval).
pub const RUN_CRITERIA: [RunCriterion; 2] = [
    RunCriterion {
        id: "task_items",
        kind: CriterionKind::Review,
        description: "Every task item is completed and accepted against its acceptance criteria.",
    },
    RunCriterion {
        id: "delivery_checks",
        kind: CriterionKind::Command,
        description: "The project's delivery checks pass at the final revision.",
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
    pub usage_budget: Option<u64>,
    /// In dependency order: each after every item it depends on.
    pub items: Vec<BriefItem>,
    /// `None`: no coordinator role, and the launcher's model is used.
    pub coordinator: Option<RoleChoice>,
    pub not_applied: Vec<NotApplied>,
}

/// A field of a team spawn cannot carry yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamField {
    Image,
    Turns,
    /// A coordinator role of more than one member.
    CoordinatorMembers,
}

impl TeamField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Turns => "turns",
            Self::CoordinatorMembers => "members",
        }
    }
}

/// Why a plan cannot brief a swarm (wording pending the owner's approval).
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
    /// A task item's title or criteria past the swarm's own bounds.
    ItemTooLarge {
        item: Slug,
    },
    BriefTooLarge {
        bytes: usize,
    },
}

impl BriefRefusal {
    /// The refusal's code, as an answer and a record name it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TeamTooLarge { .. } => "team_too_large",
            Self::TeamUnsupported { .. } => "team_unsupported",
            Self::DeadlinePassed => "deadline_passed",
            Self::ItemTooLarge { .. } => "item_too_large",
            Self::BriefTooLarge { .. } => "brief_too_large",
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
            Self::TeamUnsupported {
                field: TeamField::Image,
            } => f.write_str("a team's image is not supported yet: leave it out of the plan"),
            Self::TeamUnsupported {
                field: TeamField::Turns,
            } => f.write_str("a role's turn limit is not supported yet: leave it out of the plan"),
            Self::TeamUnsupported {
                field: TeamField::CoordinatorMembers,
            } => f.write_str("the coordinator role has one member"),
            Self::DeadlinePassed => {
                f.write_str("the team's deadline has passed: propose a plan with a later one")
            }
            Self::ItemTooLarge { item } => write!(
                f,
                "task item {} is too large for a swarm: its title must fit in \
                 {MAX_ITEM_TITLE_BYTES} bytes and its criteria in {TEXT_MAX_BYTES}",
                item.as_str()
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

// The goal always fits: its words, a slug, a version (at most three
// digits) and a task title (at most four bytes a character).
const _: () = assert!(
    "Board task , plan v: ".len() + MAX_SLUG_BYTES + 3 + 4 * MAX_TITLE_CHARS <= TEXT_MAX_BYTES
);
const _: () = assert!(MAX_PLAN_VERSION <= 999);

impl SwarmBrief {
    /// The brief's size: its swarm inputs as the swarm board's JSON spells
    /// them ([`json_text_bytes`]), and the role models it names.
    pub fn bytes(&self) -> usize {
        let criteria = criteria_bytes(&self.criteria);
        let items: usize = self
            .items
            .iter()
            .map(|item| {
                json_text_bytes(&item.title)
                    + json_list_bytes(item.acceptance.iter().map(String::as_str))
                    + json_list_bytes(item.dependencies.iter().map(Slug::as_str))
            })
            .sum();
        let models: usize = self
            .coordinator
            .iter()
            .map(|c| c.model.len())
            .sum::<usize>()
            + self
                .not_applied
                .iter()
                .map(|n| n.model.len())
                .sum::<usize>();
        json_text_bytes(&self.goal)
            + json_list_bytes(self.constraints.iter().map(String::as_str))
            + criteria
            + items
            + models
    }
}

/// The brief for `task`'s accepted `plan`, started at `now`.
///
/// # Errors
/// A team spawn cannot carry (an image, a turn limit, a coordinator role of
/// more than one member) or larger than a swarm; a deadline passed; a task
/// item past the swarm's bounds; a brief past [`MAX_BRIEF_BYTES`].
///
/// # Panics
/// `plan` is not `task`'s accepted plan: the start rule runs only that one.
pub fn brief_of(task: &Task, plan: &Plan, now: &Timestamp) -> Result<SwarmBrief, BriefRefusal> {
    let (fields, planned) = (task.fields(), plan.fields());
    assert!(
        planned.task == fields.id && fields.accepted_plan == Some(planned.version),
        "a brief is of the task's accepted plan"
    );
    let Pool {
        member_limit,
        coordinator,
        not_applied,
    } = match &planned.team {
        Some(team) => sized(team)?,
        None => Pool {
            member_limit: None,
            coordinator: None,
            not_applied: vec![],
        },
    };
    let team = planned.team.as_ref();
    let deadline_in_seconds = match team.and_then(|team| team.deadline.as_ref()) {
        Some(deadline) => match u64::try_from(deadline.seconds_since(now)) {
            Ok(left @ 1..) => left.min(MAX_DEADLINE_SECONDS),
            Ok(0) | Err(_) => return Err(BriefRefusal::DeadlinePassed),
        },
        None => DEFAULT_DEADLINE_SECONDS,
    };
    let items = in_order(&planned.items)
        .into_iter()
        .map(brief_item)
        .collect::<Result<Vec<_>, _>>()?;
    let goal = format!(
        "Board task {}, plan v{}: {}",
        fields.id.as_str(),
        planned.version,
        fields.title
    );
    assert!(
        has_content(&goal) && goal.len() <= TEXT_MAX_BYTES,
        "the goal fits"
    );
    let (constraints, approach_cut) = constraints(task, plan);
    assert!(
        criteria_bytes(&RUN_CRITERIA) <= CRITERIA_MAX_BYTES,
        "the run's criteria fit"
    );
    let brief = SwarmBrief {
        task: fields.id.clone(),
        plan_version: planned.version,
        goal,
        constraints,
        approach_cut,
        criteria: RUN_CRITERIA.to_vec(),
        member_limit,
        deadline_in_seconds,
        usage_budget: team.and_then(|team| team.budget.as_ref().map(|budget| budget.tokens)),
        items,
        coordinator,
        not_applied,
    };
    match brief.bytes() {
        bytes if bytes <= MAX_BRIEF_BYTES => Ok(brief),
        bytes => Err(BriefRefusal::BriefTooLarge { bytes }),
    }
}

/// What a team makes of the swarm's pool: its member limit, the
/// coordinator's choice, and the worker roles not applied.
struct Pool {
    member_limit: Option<u32>,
    coordinator: Option<RoleChoice>,
    not_applied: Vec<NotApplied>,
}

/// `team`'s pool, when spawn can carry it and a swarm holds it.
fn sized(team: &Team) -> Result<Pool, BriefRefusal> {
    let unsupported = |field| Err(BriefRefusal::TeamUnsupported { field });
    if team.image.is_some() {
        return unsupported(TeamField::Image);
    }
    if team.roles.iter().any(|role| role.limits.turns.is_some()) {
        return unsupported(TeamField::Turns);
    }
    let (coordinators, workers): (Vec<_>, Vec<_>) = team
        .roles
        .iter()
        .partition(|role| role.name.as_str() == COORDINATOR_ROLE);
    let coordinator = match coordinators.as_slice() {
        [] => None,
        [role] if role.limits.members == 1 => Some(RoleChoice {
            model: role.model.clone(),
            effort: role.effort,
        }),
        [_, ..] => return unsupported(TeamField::CoordinatorMembers),
    };
    let members = 1 + workers
        .iter()
        .map(|role| u32::from(role.limits.members))
        .sum::<u32>();
    if members > MAX_SWARM_MEMBERS {
        return Err(BriefRefusal::TeamTooLarge { members });
    }
    let not_applied = workers
        .iter()
        .map(|role| NotApplied {
            role: role.name.clone(),
            model: role.model.clone(),
            effort: role.effort,
        })
        .collect();
    Ok(Pool {
        member_limit: Some(members),
        coordinator,
        not_applied,
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

/// `item` as a swarm task item, within the swarm's bounds.
fn brief_item(item: &PlanItem) -> Result<BriefItem, BriefRefusal> {
    let title = format!("{}: {}", item.id.as_str(), item.title);
    let acceptance = match item.criteria.as_slice() {
        [] => vec![item.title.clone()],
        criteria => criteria.to_vec(),
    };
    debug_assert!(
        acceptance.iter().all(|line| has_content(line)),
        "a plan's lines are visible"
    );
    let fits = title.len() <= MAX_ITEM_TITLE_BYTES
        && json_list_bytes(acceptance.iter().map(String::as_str)) <= TEXT_MAX_BYTES;
    match fits {
        true => Ok(BriefItem {
            id: item.id.clone(),
            title,
            acceptance,
            dependencies: item.depends_on.clone(),
        }),
        false => Err(BriefRefusal::ItemTooLarge {
            item: item.id.clone(),
        }),
    }
}

/// The run's constraints, with whether the approach was cut: the approach,
/// if it says anything, as much of it as fits the swarm's 8 KiB, then where
/// to read the plan in full.
fn constraints(task: &Task, plan: &Plan) -> (Vec<String>, bool) {
    let pointer = format!(
        "The plan in full, approach included: the board's show op for task {} (plan v{}).",
        task.fields().id.as_str(),
        plan.fields().version
    );
    let approach = &plan.fields().approach;
    if !has_content(approach) {
        return (vec![pointer], false);
    }
    // Room for the approach's characters: the list's brackets and comma,
    // the pointer, and the approach's own quotes.
    let room = TEXT_MAX_BYTES - 2 - 1 - json_text_bytes(&pointer) - 2;
    let mut used = 0;
    let kept: String = approach
        .chars()
        .take_while(|c| {
            used += char_bytes(*c);
            used <= room
        })
        .collect();
    let cut = kept.len() < approach.len();
    let constraints = vec![kept, pointer];
    assert!(
        json_list_bytes(constraints.iter().map(String::as_str)) <= TEXT_MAX_BYTES,
        "the constraints fit"
    );
    (constraints, cut)
}

/// `criteria`'s bytes as the swarm board's JSON spells the list: each an
/// object of its `description`, `id` and `kind`.
fn criteria_bytes(criteria: &[RunCriterion]) -> usize {
    let field = |key: &str, value: &str| json_text_bytes(key) + 1 + json_text_bytes(value);
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

/// One character's bytes in the swarm board's JSON: Python's `json.dumps`
/// with `ensure_ascii`, so only printable ASCII is itself.
fn char_bytes(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        ' '..='~' => 1,
        '\u{0}'..='\u{ffff}' => 6,
        _ => 12,
    }
}

/// `text`'s bytes as a JSON string in the swarm board's encoding.
fn json_text_bytes(text: &str) -> usize {
    2 + text.chars().map(char_bytes).sum::<usize>()
}

/// `texts`' bytes as a JSON list of strings in the swarm board's encoding
/// (no spaces between items).
fn json_list_bytes<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    let (count, bytes) = texts
        .into_iter()
        .fold((0_usize, 0_usize), |(count, bytes), text| {
            (count + 1, bytes + json_text_bytes(text))
        });
    2 + bytes + count.saturating_sub(1)
}

#[cfg(test)]
#[path = "swarm_brief_tests.rs"]
mod tests;
