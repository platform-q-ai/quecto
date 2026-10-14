//! A run: one swarm execution of one plan version (#2481's artifact
//! model). It is opened when the run starts and folded back once when it
//! ends: its outcome, cost, and where each of the plan's task items stands with
//! its evidence. A [`Run`] is only ever built from fields that pass
//! [`Run::new`].
use super::super::value_objects::schema_error::{SchemaError, distinct, line};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::PlanState;
use super::super::value_objects::vocabulary::{ItemState, RunOutcome};
use super::plan::{MAX_PLAN_VERSION, Plan};
use super::task_parts::{MAX_BUDGET_TOKENS, MAX_ITEMS, MAX_OWNER_CHARS, MAX_PER_ITEM, lines};

/// The most runs a task starts.
pub const MAX_RUN_NUMBER: u32 = 999;
/// All of a run's text together, in UTF-8 bytes: its task items' owners and
/// evidence. Runs hold summaries and links, never logs; this budget keeps
/// every run inside the one file it is stored in, with room to spare (its
/// codec, M2, proves the fit).
pub const MAX_RUN_TEXT_BYTES: usize = 256 * 1024;

/// Where one of the plan's task items stood when the run folded back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunItem {
    /// The task item's id.
    pub id: Slug,
    pub state: ItemState,
    pub owner: Option<String>,
    /// Summaries and links, never logs.
    pub evidence: Vec<String>,
}

/// A run's content, unchecked: what [`Run::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFields {
    /// The task it ran for.
    pub task: Slug,
    /// From 1, per task.
    pub number: u32,
    /// The plan version it ran, exactly.
    pub plan_version: u32,
    pub started: Timestamp,
    /// With `outcome`: the run has folded back.
    pub ended: Option<Timestamp>,
    pub outcome: Option<RunOutcome>,
    /// Tokens the run spent, once known.
    pub cost: Option<u64>,
    pub items: Vec<RunItem>,
    /// The environment its swarm's coordinator runs in (`C` and its number,
    /// #2481 S4d), once the session that started it keeps it: what a later
    /// session of the same holder reads to take the keeping up again.
    pub environment: Option<String>,
}

/// The most digits of an environment's number a run names.
const MAX_ENVIRONMENT_DIGITS: usize = 9;

/// Whether `environment` is an environment ref a run names: `C`, then 1
/// to 9 digits, the first not `0`.
pub fn is_environment_ref(environment: &str) -> bool {
    environment.strip_prefix('C').is_some_and(|digits| {
        (1..=MAX_ENVIRONMENT_DIGITS).contains(&digits.len())
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !digits.starts_with('0')
    })
}

/// A run whose fields keep every rule of its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run(RunFields);

impl RunFields {
    /// The UTF-8 bytes of every free text in the run: its items' owners
    /// and evidence.
    pub fn text_bytes(&self) -> usize {
        self.items
            .iter()
            .map(|item| {
                item.owner.as_ref().map_or(0, String::len)
                    + item.evidence.iter().map(String::len).sum::<usize>()
            })
            .sum()
    }
}

impl Run {
    pub fn new(fields: RunFields) -> Result<Self, SchemaError> {
        validate(&fields)?;
        Ok(Self(fields))
    }

    pub fn fields(&self) -> &RunFields {
        &self.0
    }

    pub fn into_fields(self) -> RunFields {
        self.0
    }

    /// Still going: not folded back.
    pub fn is_open(&self) -> bool {
        self.0.ended.is_none()
    }

    /// The run ran `plan`: an accepted plan (or one superseded since), of
    /// the same task and version, and every item it reports is one of the
    /// plan's.
    ///
    /// # Errors
    /// The field that names a plan never accepted, or another task,
    /// version or task item.
    pub fn ran(&self, plan: &Plan) -> Result<(), SchemaError> {
        let planned = plan.fields();
        match planned.state {
            PlanState::Accepted | PlanState::Superseded => {}
            state @ (PlanState::Draft | PlanState::Discarded) => {
                return Err(SchemaError::new(
                    "plan",
                    format!("a {} plan never ran", state.as_str()),
                ));
            }
        }
        match (
            planned.task == self.0.task,
            planned.version == self.0.plan_version,
        ) {
            (true, true) => {}
            (false, _) => return Err(SchemaError::new("task", "is not the plan's task")),
            (true, false) => {
                return Err(SchemaError::new(
                    "plan_version",
                    "is not the plan's version",
                ));
            }
        }
        match self
            .0
            .items
            .iter()
            .position(|item| plan.item(&item.id).is_none())
        {
            Some(index) => Err(SchemaError::new(
                format!("items/{index}/id"),
                "is not a task item of the plan",
            )),
            None => Ok(()),
        }
    }
}

impl RunItem {
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        if let Some(owner) = &self.owner {
            line(&format!("{field}/owner"), owner, MAX_OWNER_CHARS)?;
        }
        lines(&format!("{field}/evidence"), &self.evidence, MAX_PER_ITEM)
    }
}

/// Numbered from 1 against plan versions from 1; open, or ended with an
/// outcome no earlier than it started (a run can end within the second it
/// starts); a cost and item states only once it has ended.
fn validate(run: &RunFields) -> Result<(), SchemaError> {
    match (1..=MAX_RUN_NUMBER).contains(&run.number) {
        true => {}
        false => {
            return Err(SchemaError::new(
                "number",
                format!("must be 1 to {MAX_RUN_NUMBER}"),
            ));
        }
    }
    match (1..=MAX_PLAN_VERSION).contains(&run.plan_version) {
        true => {}
        false => {
            return Err(SchemaError::new(
                "plan_version",
                format!("must be 1 to {MAX_PLAN_VERSION}"),
            ));
        }
    }
    let ended = match (&run.ended, &run.outcome) {
        (None, None) => false,
        (Some(ended), Some(_)) if *ended >= run.started => true,
        _ => {
            return Err(SchemaError::new(
                "ended",
                "an ended run has an outcome and ends no earlier than it starts",
            ));
        }
    };
    match (ended, run.cost.is_some(), run.items.is_empty()) {
        (true, _, _) | (false, false, true) => {}
        (false, true, _) => {
            return Err(SchemaError::new(
                "cost",
                "a run reports its cost when it folds back",
            ));
        }
        (false, false, false) => {
            return Err(SchemaError::new(
                "items",
                "a run reports its task items when it folds back",
            ));
        }
    }
    match run.cost.is_none_or(|tokens| tokens <= MAX_BUDGET_TOKENS) {
        true => {}
        false => {
            return Err(SchemaError::new(
                "cost",
                format!("must be at most {MAX_BUDGET_TOKENS}"),
            ));
        }
    }
    match run.environment.as_deref().is_none_or(is_environment_ref) {
        true => {}
        false => {
            return Err(SchemaError::new(
                "environment",
                "must be C and 1 to 9 digits, the first not 0",
            ));
        }
    }
    let ids: Vec<&Slug> = run.items.iter().map(|item| &item.id).collect();
    distinct("items", &ids, MAX_ITEMS)?;
    for (index, item) in run.items.iter().enumerate() {
        item.validate(&format!("items/{index}"))?;
    }
    let text = run.text_bytes();
    match text <= MAX_RUN_TEXT_BYTES {
        true => Ok(()),
        false => Err(SchemaError::new(
            "text",
            format!("the run's text is {text} bytes; the most is {MAX_RUN_TEXT_BYTES}"),
        )),
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
