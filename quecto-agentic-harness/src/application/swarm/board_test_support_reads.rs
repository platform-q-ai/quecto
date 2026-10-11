//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! read-model reads (#2277): the run as `dict(row)`, member statuses, the
//! grouped latest activity (each call journalled, so a test counts the
//! scans), event times and pages.
use serde_json::{Value, json};

use super::{BoardState, RecordedEvent, ids::sorted};
use crate::application::swarm::dto::{DictRow, LatestActivity, MemberRow};

/// `SELECT * FROM run` as `dict(row)`, the board's columns in order.
pub(super) fn run_row(state: &BoardState) -> Option<DictRow> {
    let run = state.run.as_ref()?;
    let text = |value: &Option<String>| value.clone().map_or(Value::Null, Value::String);
    let record = &run.record;
    Some(DictRow {
        columns: vec![
            ("id".to_owned(), json!(run.id)),
            ("goal".to_owned(), json!(run.contract.goal)),
            ("constraints".to_owned(), run.contract.constraints.clone()),
            ("criteria".to_owned(), run.contract.criteria.clone()),
            ("coordinator".to_owned(), text(&record.coordinator)),
            ("integrator".to_owned(), text(&record.coordinator)),
            ("member_limit".to_owned(), json!(record.member_limit)),
            ("deadline".to_owned(), json!(record.deadline)),
            (
                "status".to_owned(),
                record
                    .status
                    .as_ref()
                    .map_or(Value::Null, |status| json!(status.as_str())),
            ),
            ("outcome".to_owned(), text(&record.outcome)),
            ("outcome_reason".to_owned(), text(&record.outcome_reason)),
        ],
    })
}

/// `SELECT id,status FROM members WHERE id IN (…)`.
pub(super) fn member_statuses(
    members: &[MemberRow],
    ids: &[&str],
) -> Vec<(String, Option<String>)> {
    members
        .iter()
        .filter_map(|row| {
            let id = row.text("id")?;
            ids.contains(&id)
                .then(|| (id.to_owned(), row.text("status").map(str::to_owned)))
        })
        .collect()
}

/// The grouped `max(time)` of each of `actors`' events.
pub(super) fn latest_activity(events: &[RecordedEvent], actors: &[&str]) -> Vec<LatestActivity> {
    actors
        .iter()
        .filter_map(|actor| {
            let latest = events
                .iter()
                .filter(|event| event.actor == *actor)
                .map(|event| event.time)
                .reduce(f64::max)?;
            Some(LatestActivity {
                actor: (*actor).to_owned(),
                latest: json!(latest),
            })
        })
        .collect()
}

/// Event `id`'s time (ids count from 1 in write order).
pub(super) fn event_time(events: &[RecordedEvent], id: i64) -> Option<Value> {
    let index = usize::try_from(id).ok()?.checked_sub(1)?;
    events.get(index).map(|event| json!(event.time))
}

/// `SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?`, each detail as
/// the board's sorted, compact text.
pub(super) fn event_page(events: &[RecordedEvent], after: u64, limit: i64) -> Vec<DictRow> {
    let skip = usize::try_from(after).unwrap_or(usize::MAX);
    let take = usize::try_from(limit).unwrap_or(0);
    events
        .iter()
        .enumerate()
        .skip(skip)
        .take(take)
        .map(|(index, event)| DictRow {
            columns: vec![
                ("id".to_owned(), json!(index + 1)),
                ("actor".to_owned(), json!(event.actor)),
                ("time".to_owned(), json!(event.time)),
                ("action".to_owned(), json!(event.action)),
                (
                    "detail".to_owned(),
                    json!(serde_json::to_string(&sorted(&event.detail)).unwrap()),
                ),
            ],
        })
        .collect()
}

/// The projection reads (#2478) over the in-memory board: the allowlisted
/// columns, each kind sorted as the SQL sorts it and cut at its limit.
impl crate::application::swarm::ports::BoardProjectionReads for super::MemoryTransaction<'_> {
    fn projection_revision(&self) -> Result<i64, crate::domain::swarm::BoardError> {
        let state = self.state.borrow();
        Ok(i64::try_from(state.events.len() + state.request_usage.len()).unwrap())
    }

    fn projection_rows(
        &self,
        limits: crate::application::swarm::dto::ProjectionLimits,
    ) -> Result<crate::application::swarm::dto::ProjectionRows, crate::domain::swarm::BoardError>
    {
        use crate::application::swarm::dto::{
            ProjectionRows, StoredEvidenceColumns, StoredItemColumns, StoredMemberColumns,
            StoredRunColumns,
        };
        let state = self.state.borrow();
        let cut = |limit: i64| usize::try_from(limit).unwrap();
        let run = run_row(&state).map(|row| {
            let column = |name: &str| row.get(name).cloned().unwrap_or(Value::Null);
            StoredRunColumns {
                id: column("id"),
                goal: column("goal"),
                criteria: column("criteria"),
                coordinator: column("coordinator"),
                integrator: column("integrator"),
                member_limit: column("member_limit"),
                deadline: column("deadline"),
                status: column("status"),
                outcome: column("outcome"),
                outcome_reason: column("outcome_reason"),
            }
        });
        let mut tasks = state.tasks.clone();
        tasks.sort_by_key(|task| task.get("id").and_then(Value::as_i64));
        let items = tasks
            .iter()
            .take(cut(limits.items))
            .map(|task| {
                let column = |name: &str| task.get(name).cloned().unwrap_or(Value::Null);
                StoredItemColumns {
                    id: column("id"),
                    title: column("title"),
                    criteria: column("acceptance"),
                    depends_on: column("dependencies"),
                    status: column("status"),
                    owner: column("owner"),
                    blocker: column("blocker"),
                    evidence: column("evidence"),
                }
            })
            .collect();
        let mut files: Vec<(Value, Value)> = state
            .files
            .iter()
            .map(|file| (json!(file.path), json!(file.task)))
            .collect();
        files.sort_by(|left, right| left.0.as_str().cmp(&right.0.as_str()));
        let mut members = state.members.clone();
        members.sort_by(|left, right| left.text("id").cmp(&right.text("id")));
        let names: Vec<&str> = members
            .iter()
            .filter_map(|member| member.text("id"))
            .collect();
        let latest = latest_activity(&state.events, &names);
        let members = members
            .iter()
            .take(cut(limits.members))
            .map(|member| {
                let id = member.get("id").cloned().unwrap_or(Value::Null);
                let last_activity = latest
                    .iter()
                    .find(|activity| Some(activity.actor.as_str()) == id.as_str())
                    .map_or(Value::Null, |activity| activity.latest.clone());
                StoredMemberColumns {
                    status: member.get("status").cloned().unwrap_or(Value::Null),
                    last_activity,
                    id,
                }
            })
            .collect();
        let mut evidence = state.evidence.clone();
        evidence.sort_by(|left, right| {
            (left.1.criterion.as_str(), left.0.as_str())
                .cmp(&(right.1.criterion.as_str(), right.0.as_str()))
        });
        let evidence = evidence
            .iter()
            .take(cut(limits.evidence))
            .map(|(actor, entry)| StoredEvidenceColumns {
                criterion: entry.criterion.clone(),
                artifact: entry.artifact.clone(),
                revision: entry.revision.clone(),
                kind: entry.kind.clone(),
                actor: json!(actor),
                accepted: entry.accepted.clone(),
            })
            .collect();
        let total = |key: &str| {
            state
                .usage
                .as_ref()
                .and_then(|usage| usage.totals.columns.iter().find(|(name, _)| name == key))
                .map_or(json!(0), |(_, value)| value.clone())
        };
        let generation = state
            .events
            .iter()
            .rposition(|event| matches!(event.action.as_str(), "paused" | "resumed"))
            .map_or(0, |index| i64::try_from(index + 1).unwrap());
        Ok(ProjectionRows {
            revision: i64::try_from(state.events.len() + state.request_usage.len()).unwrap(),
            generation,
            run,
            items,
            item_count: i64::try_from(state.tasks.len()).unwrap(),
            file_count: i64::try_from(files.len()).unwrap(),
            files: files.into_iter().take(cut(limits.files)).collect(),
            members,
            member_count: i64::try_from(state.members.len()).unwrap(),
            evidence,
            evidence_count: i64::try_from(state.evidence.len()).unwrap(),
            budget: state.usage.as_ref().map(|usage| usage.budget.clone()),
            observed_tokens: total("observed_tokens"),
            unknown_usage_requests: total("unknown_usage_requests"),
        })
    }
}
