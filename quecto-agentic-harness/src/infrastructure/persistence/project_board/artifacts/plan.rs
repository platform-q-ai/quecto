//! A plan's file (`quecto-plan`, epic #2481 M2b-3). Its title is derived,
//! `Plan {version} of {task}`; its fields and tables hold the rest, every
//! list in the plan's own order:
//!
//! - Items `[Item | Title]`, in item order;
//! - Item dependencies `[Item | Depends on]`, one row per edge, in item
//!   order then dependency order;
//! - Criteria `[Item | Criterion]`, grouped in item order;
//! - Roles `[Role | Model | Effort | Members | Turns]`, present exactly
//!   when the plan names a team (Turns `-` for no limit). The team's
//!   budget, deadline and image rows are taken only with Roles.
//!
//! A missing field row or section is "not set" or "none". The Model and
//! Image cells are code: a model or image reference may hold a `_` an
//! ASCII cell refuses next to punctuation.
use super::cells::{self, column, refused};
use super::{ArtifactCodec, CodecError};
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::team::{Budget, Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::vocabulary::{Effort, PlanState};
use crate::infrastructure::persistence::project_board::markdown::{
    Body, BodyShape, CellKind, Column, Document, Limits, Section, SectionShape, Shape, blocks_of,
    text_of,
};

const TASK: &str = "Task";
const VERSION: &str = "Version";
const STATE: &str = "State";
const PROPOSED_BY: &str = "Proposed by";
const EDITED_BY: &str = "Edited by";
const ACCEPTED_BY: &str = "Accepted by";
const ACCEPTED_AT: &str = "Accepted at";
const BUDGET: &str = "Budget tokens";
const DEADLINE: &str = "Deadline";
const IMAGE: &str = "Image";
const CREATED: &str = "Created";
const UPDATED: &str = "Updated";
const APPROACH: &str = "Approach";
const ITEMS: &str = "Items";
const DEPENDENCIES: &str = "Item dependencies";
const CRITERIA: &str = "Criteria";
const ROLES: &str = "Roles";

const FIELDS: &[Column<'static>] = &[
    column(TASK, CellKind::Ascii),
    column(VERSION, CellKind::Ascii),
    column(STATE, CellKind::Ascii),
    column(PROPOSED_BY, CellKind::Text),
    column(EDITED_BY, CellKind::Text),
    column(ACCEPTED_BY, CellKind::Text),
    column(ACCEPTED_AT, CellKind::Ascii),
    column(BUDGET, CellKind::Ascii),
    column(DEADLINE, CellKind::Ascii),
    column(IMAGE, CellKind::Code),
    column(CREATED, CellKind::Ascii),
    column(UPDATED, CellKind::Ascii),
];

const ITEM_COLUMNS: &[Column<'static>] = &[
    column("Item", CellKind::Ascii),
    column("Title", CellKind::Text),
];
const DEPENDENCY_COLUMNS: &[Column<'static>] = &[
    column("Item", CellKind::Ascii),
    column("Depends on", CellKind::Ascii),
];
const CRITERION_COLUMNS: &[Column<'static>] = &[
    column("Item", CellKind::Ascii),
    column("Criterion", CellKind::Text),
];
const ROLE_COLUMNS: &[Column<'static>] = &[
    column("Role", CellKind::Ascii),
    column("Model", CellKind::Code),
    column("Effort", CellKind::Ascii),
    column("Members", CellKind::Ascii),
    column("Turns", CellKind::Ascii),
];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: APPROACH,
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: ITEMS,
        body: BodyShape::Table(ITEM_COLUMNS),
    },
    SectionShape {
        heading: DEPENDENCIES,
        body: BodyShape::Table(DEPENDENCY_COLUMNS),
    },
    SectionShape {
        heading: CRITERIA,
        body: BodyShape::Table(CRITERION_COLUMNS),
    },
    SectionShape {
        heading: ROLES,
        body: BodyShape::Table(ROLE_COLUMNS),
    },
];

/// A plan's file caps: the widest plan the domain admits fits
/// (`the_widest_plan_fits_its_limits`). An approach of 65,536 characters
/// may be that many lines inside a fence, so the line cap is the
/// approach's lines and every table at its fullest.
pub const PLAN_LIMITS: Limits = Limits {
    file_bytes: 1_536 * 1024,
    lines: 70_000,
    line_bytes: 264 * 1024,
};

pub struct PlanCodec;

impl ArtifactCodec for PlanCodec {
    type Value = Plan;
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "plan",
        fields: FIELDS,
        sections: SECTIONS,
        limits: PLAN_LIMITS,
    };

    fn to_document(plan: &Plan) -> Result<Document, CodecError> {
        let plan = plan.fields();
        let mut fields = vec![
            (TASK, cells::slug(&plan.task)),
            (VERSION, cells::number(plan.version.into())),
            (STATE, cells::word(plan.state.as_str())),
            (
                PROPOSED_BY,
                cells::identity(PROPOSED_BY, &plan.proposed_by)?,
            ),
        ];
        if let Some(editor) = &plan.edited_by {
            fields.push((EDITED_BY, cells::identity(EDITED_BY, editor)?));
        }
        if let Some((by, at)) = &plan.accepted {
            fields.push((ACCEPTED_BY, cells::identity(ACCEPTED_BY, by)?));
            fields.push((ACCEPTED_AT, cells::time(at)));
        }
        if let Some(team) = &plan.team {
            let budget = team
                .budget
                .as_ref()
                .map(|budget| cells::number(budget.tokens));
            let deadline = team.deadline.as_ref().map(cells::time);
            let held = [
                (BUDGET, budget),
                (DEADLINE, deadline),
                (IMAGE, team.image.clone()),
            ];
            fields.extend(
                held.into_iter()
                    .filter_map(|(name, cell)| Some((name, cell?))),
            );
        }
        fields.push((CREATED, cells::time(&plan.created)));
        fields.push((UPDATED, cells::time(&plan.updated)));
        let mut sections = vec![];
        // The approach's own line, from 1, names where it is refused.
        let approach =
            blocks_of(&plan.approach).map_err(|error| refused(APPROACH, error.to_string()))?;
        if let Some(approach) = held(approach) {
            sections.push(section(APPROACH, Body::Prose(approach)));
        }
        let ids = |item: &PlanItem| cells::slug(&item.id);
        let items = plan
            .items
            .iter()
            .map(|item| vec![ids(item), item.title.clone()]);
        let dependencies = plan.items.iter().flat_map(|item| {
            item.depends_on
                .iter()
                .map(move |on| vec![ids(item), cells::slug(on)])
        });
        let criteria = plan.items.iter().flat_map(|item| {
            item.criteria
                .iter()
                .map(move |criterion| vec![ids(item), criterion.clone()])
        });
        for (heading, rows) in [
            (ITEMS, items.collect::<Vec<_>>()),
            (DEPENDENCIES, dependencies.collect()),
            (CRITERIA, criteria.collect()),
        ] {
            if let Some(rows) = held(rows) {
                sections.push(section(heading, Body::Table(rows)));
            }
        }
        if let Some(team) = &plan.team {
            let rows = team.roles.iter().map(|role| {
                let turns = role.limits.turns.map(|turns| cells::number(turns.into()));
                Ok(vec![
                    cells::slug(&role.name),
                    role.model.clone(),
                    cells::word(role.effort.as_str()),
                    cells::number(role.limits.members.into()),
                    cells::optional(ROLES, turns)?,
                ])
            });
            let rows = rows.collect::<Result<Vec<_>, CodecError>>()?;
            sections.push(section(ROLES, Body::Table(rows)));
        }
        debug_assert!(
            in_shape_order(fields.iter().map(|(name, _)| *name)),
            "the fields are written in the shape's order"
        );
        debug_assert_eq!(
            plan.team.is_some(),
            sections.iter().any(|section| section.heading == ROLES),
            "a Roles section exactly when the plan names a team"
        );
        Ok(Document {
            schema: Self::SCHEMA,
            title: title(plan),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections,
        })
    }

    fn from_document(document: &Document) -> Result<Plan, CodecError> {
        let fields = cells::Fields::of(document);
        let field = |name: &str| fields.get(name);
        let required = |name: &'static str| fields.required(name);
        let version = cells::read_number(VERSION, required(VERSION)?)?;
        let version = u32::try_from(version).map_err(|_| refused(VERSION, "past u32"))?;
        let accepted = match (field(ACCEPTED_BY), field(ACCEPTED_AT)) {
            (Some(by), Some(at)) => Some((
                cells::read_identity(ACCEPTED_BY, by)?,
                cells::read_time(ACCEPTED_AT, at)?,
            )),
            (None, None) => None,
            _ => {
                return Err(refused(
                    ACCEPTED_BY,
                    "both or neither of it and Accepted at",
                ));
            }
        };
        let mut read = Read::default();
        for section in &document.sections {
            read.section(section)?;
        }
        let team = match read.roles {
            Some(roles) => Some(Team {
                roles,
                budget: match field(BUDGET) {
                    Some(cell) => Some(Budget {
                        tokens: cells::read_number(BUDGET, cell)?,
                    }),
                    None => None,
                },
                deadline: field(DEADLINE)
                    .map(|cell| cells::read_time(DEADLINE, cell))
                    .transpose()?,
                image: field(IMAGE).map(str::to_string),
            }),
            None => match [BUDGET, DEADLINE, IMAGE]
                .into_iter()
                .find(|name| field(name).is_some())
            {
                Some(name) => return Err(refused(name, "only with a team (a Roles section)")),
                None => None,
            },
        };
        let fields = PlanFields {
            task: cells::read_slug(TASK, required(TASK)?)?,
            version,
            state: cells::read_word(STATE, required(STATE)?, PlanState::parse)?,
            approach: read.approach,
            items: read.items,
            team,
            proposed_by: cells::read_identity(PROPOSED_BY, required(PROPOSED_BY)?)?,
            edited_by: field(EDITED_BY)
                .map(|cell| cells::read_identity(EDITED_BY, cell))
                .transpose()?,
            accepted,
            created: cells::read_time(CREATED, required(CREATED)?)?,
            updated: cells::read_time(UPDATED, required(UPDATED)?)?,
        };
        Plan::new(fields)
            .map_err(|error| refused("Plan", format!("{}: {}", error.field, error.problem)))
    }
}

/// Whether `names` come in [`FIELDS`]' order, each once.
fn in_shape_order<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut shape = FIELDS.iter();
    names
        .into_iter()
        .all(|name| shape.any(|field| field.name == name))
}

/// The derived title.
fn title(plan: &PlanFields) -> String {
    format!("Plan {} of {}", plan.version, plan.task.as_str())
}

/// `entries`, when it holds any: a section only for what holds something
/// (a missing one is "none").
fn held<T>(entries: Vec<T>) -> Option<Vec<T>> {
    match entries.as_slice() {
        [_, ..] => Some(entries),
        [] => None,
    }
}

fn section(heading: &str, body: Body) -> Section {
    Section {
        heading: heading.into(),
        body,
    }
}

/// What a plan's sections hold, read in the shape's order.
#[derive(Default)]
struct Read {
    approach: String,
    items: Vec<PlanItem>,
    roles: Option<Vec<Role>>,
}

impl Read {
    fn section(&mut self, section: &Section) -> Result<(), CodecError> {
        match (section.heading.as_str(), &section.body) {
            (APPROACH, Body::Prose(blocks)) => self.approach = text_of(blocks),
            (ITEMS, Body::Table(rows)) => {
                for row in rows {
                    let [id, title] = row.as_slice() else {
                        return Err(refused(ITEMS, "a row of two cells"));
                    };
                    self.items.push(PlanItem {
                        id: cells::read_slug(ITEMS, id)?,
                        title: title.clone(),
                        criteria: vec![],
                        depends_on: vec![],
                    });
                }
            }
            (DEPENDENCIES, Body::Table(rows)) => {
                for row in rows {
                    let [id, on] = row.as_slice() else {
                        return Err(refused(DEPENDENCIES, "a row of two cells"));
                    };
                    let on = cells::read_slug(DEPENDENCIES, on)?;
                    // Items are read before their dependencies.
                    match self.items.iter().any(|item| item.id == on) {
                        true => self.item(DEPENDENCIES, id)?.depends_on.push(on),
                        false => {
                            return Err(refused(
                                DEPENDENCIES,
                                format!("{} is no item of the plan", on.as_str()),
                            ));
                        }
                    }
                }
            }
            (CRITERIA, Body::Table(rows)) => {
                for row in rows {
                    let [id, criterion] = row.as_slice() else {
                        return Err(refused(CRITERIA, "a row of two cells"));
                    };
                    self.item(CRITERIA, id)?.criteria.push(criterion.clone());
                }
            }
            (ROLES, Body::Table(rows)) => {
                let roles = rows.iter().map(|row| role(row)).collect::<Result<_, _>>()?;
                self.roles = Some(roles);
            }
            _ => return Err(refused("Plan", format!("a section {:?}", section.heading))),
        }
        Ok(())
    }

    /// The item `id` names, among the items read so far.
    fn item(&mut self, table: &'static str, id: &str) -> Result<&mut PlanItem, CodecError> {
        let id = cells::read_slug(table, id)?;
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| refused(table, format!("{} is no item of the plan", id.as_str())))
    }
}

fn role(row: &[String]) -> Result<Role, CodecError> {
    let [name, model, effort, members, turns] = row else {
        return Err(refused(ROLES, "a row of five cells"));
    };
    let members = cells::read_number(ROLES, members)?;
    let turns = match cells::read_optional(turns) {
        Some(turns) => Some(cells::read_number(ROLES, turns)?),
        None => None,
    };
    let narrow = |_| refused(ROLES, "members or turns past their range");
    Ok(Role {
        name: cells::read_slug(ROLES, name)?,
        model: model.clone(),
        effort: cells::read_word(ROLES, effort, Effort::parse)?,
        limits: RoleLimits {
            members: u16::try_from(members).map_err(narrow)?,
            turns: turns.map(u32::try_from).transpose().map_err(narrow)?,
        },
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
