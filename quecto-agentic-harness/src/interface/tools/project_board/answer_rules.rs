//! A board rule's refusal as an answer carries it (`answer`): its code,
//! the one a call's record holds too, and the rule's own fields.
use serde_json::{Map, Value, json};

use super::shown;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;

/// A board rule's refusal: `{"code": …}` and the rule's own fields.
pub(super) fn rule_json(rule: &BoardRuleError) -> Value {
    let mut answer = Map::new();
    answer.insert("code".into(), Value::String(rule_code(rule).into()));
    answer.extend(rule_fields(rule));
    Value::Object(answer)
}

/// A board rule's code, as its refusal answers it and a call's record
/// holds it (epic #2481 T3). Exhaustive, so a new rule does not compile
/// until it has its code.
pub fn rule_code(rule: &BoardRuleError) -> &'static str {
    use BoardRuleError as R;
    match rule {
        R::NotAllowed { .. } => "not_allowed",
        R::NotHolder => "not_holder",
        R::NoClaim => "no_claim",
        R::ClaimLive => "claim_live",
        R::ClaimExpired => "claim_expired",
        R::RunOpen(..) => "run_open",
        R::RunEnded(..) => "run_ended",
        R::PlanNotDraft(..) => "plan_not_draft",
        R::NotThisTasks => "not_this_tasks",
        R::NotThisChange => "not_this_change",
        R::TaskFinished(..) => "task_finished",
        R::UnknownPlanVersion(..) => "unknown_plan_version",
        R::PlanChanged { .. } => "plan_changed",
        R::PlansInconsistent(..) => "plans_inconsistent",
        R::PlanAhead { .. } => "plan_ahead",
        R::RunsInconsistent(..) => "runs_inconsistent",
        R::PlanNotAccepted(..) => "plan_not_accepted",
        R::NoAcceptedPlan => "no_accepted_plan",
        R::ChangeAlreadyLinked { .. } => "change_already_linked",
        R::NoSuchChange(..) => "no_such_change",
        R::NoSuchRun(..) => "no_such_run",
        R::ChangeAhead { .. } => "change_ahead",
        R::DiscussionClosed(..) => "discussion_closed",
        R::DiscussionLinked { .. } => "discussion_linked",
        R::DiscussionAhead { .. } => "discussion_ahead",
        R::MessageAhead { .. } => "message_ahead",
        R::DiscussionFull { .. } => "discussion_full",
        R::ReviewAhead { .. } => "review_ahead",
        R::ChangesLinked(..) => "changes_linked",
        R::PrUnread(..) => "pr_unread",
        R::PrNotOpen(..) => "pr_not_open",
        R::PrHeadMismatch(..) => "pr_head_mismatch",
        R::ChangeUnlinked(..) => "change_unlinked",
        R::ChangeMerged(..) => "change_merged",
        R::NoChanges => "no_changes",
        R::ChangesUnmerged(..) => "changes_unmerged",
        R::ChangesInconsistent(..) => "changes_inconsistent",
        R::NotTakeable(..) => "not_takeable",
        R::AlreadyHolder => "already_holder",
        R::NoPrs => "no_prs",
        R::PrsUnmerged(..) => "prs_unmerged",
        R::PrsLinked(..) => "prs_linked",
        R::PrNotLinked(..) => "pr_not_linked",
        R::PrAlreadyLinked(..) => "pr_already_linked",
        R::NotDraft(..) => "not_draft",
        R::NoEdit => "no_edit",
        R::Superseded { .. } => "superseded",
        R::IssueShared { .. } => "issue_shared",
        R::IssueIsPr { .. } => "issue_is_pr",
        R::OwnPrReferenced { .. } => "own_pr_referenced",
        R::Invalid(..) => "invalid",
        R::InvalidPlan(..) => "invalid_plan",
    }
}

/// A JSON object's fields, in order: `object!({"key": value, ...})`, each
/// value as `json!` reads it.
macro_rules! object {
    ({$($key:literal: $value:expr),* $(,)?}) => {{
        let mut fields = Map::new();
        $(fields.insert($key.to_string(), json!($value));)*
        fields
    }};
}

/// A board rule's own fields, its code aside.
fn rule_fields(rule: &BoardRuleError) -> Map<String, Value> {
    use BoardRuleError as R;
    match rule {
        R::NotAllowed { from, to } => {
            object!({"from": from.as_str(), "to": to.as_str()})
        }
        R::NotHolder => Map::new(),
        R::NoClaim => Map::new(),
        R::ClaimLive => Map::new(),
        R::ClaimExpired => Map::new(),
        R::RunOpen(run) => object!({"run": run}),
        R::RunEnded(run) => object!({"run": run}),
        R::PlanNotDraft(state) => object!({"state": state.as_str()}),
        R::NotThisTasks => Map::new(),
        R::NotThisChange => Map::new(),
        R::TaskFinished(status) => object!({"status": status.as_str()}),
        R::UnknownPlanVersion(version) => {
            object!({"version": version})
        }
        R::PlanChanged { version, updated } => object!({
            "version": version,
            "updated": updated.as_str(),
        }),
        R::PlansInconsistent(why) => object!({"reason": shown(why)}),
        R::PlanAhead { version, updated } => object!({
            "version": version,
            "updated": updated.as_str(),
        }),
        R::RunsInconsistent(why) => object!({"reason": shown(why)}),
        R::PlanNotAccepted(version) => object!({"version": version}),
        R::NoAcceptedPlan => Map::new(),
        R::ChangeAlreadyLinked { task, change } => {
            object!({"task": task.as_str(), "change": change})
        }
        R::NoSuchChange(change) => object!({"change": change}),
        R::NoSuchRun(run) => object!({"run": run}),
        R::ChangeAhead { change, updated } => object!({
            "change": change,
            "updated": updated.as_str(),
        }),
        R::DiscussionClosed(id) => object!({"discussion": id.as_str()}),
        R::DiscussionLinked { discussion, task } => object!({
            "discussion": discussion.as_str(),
            "task": task.as_str(),
        }),
        R::DiscussionAhead {
            discussion,
            updated,
        } => object!({
            "discussion": discussion.as_str(),
            "updated": updated.as_str(),
        }),
        R::MessageAhead { discussion, at } => object!({
            "discussion": discussion.as_str(),
            "at": at.as_str(),
        }),
        R::DiscussionFull { discussion, limit } => object!({
            "discussion": discussion.as_str(),
            "limit": limit.as_str(),
        }),
        R::ReviewAhead { change, updated } => object!({
            "change": change,
            "updated": updated.as_str(),
        }),
        R::ChangesLinked(changes) => object!({"changes": changes}),
        R::PrUnread(pr) => object!({"pr": pr}),
        R::PrNotOpen(pr) => object!({"pr": pr}),
        R::PrHeadMismatch(pr) => object!({"pr": pr}),
        R::ChangeUnlinked(change) => object!({"change": change}),
        R::ChangeMerged(change) => object!({"change": change}),
        R::NoChanges => Map::new(),
        R::ChangesUnmerged(changes) => object!({"changes": changes}),
        R::ChangesInconsistent(why) => {
            object!({"reason": shown(why)})
        }
        R::NotTakeable(status) => object!({"status": status.as_str()}),
        R::AlreadyHolder => Map::new(),
        R::NoPrs => Map::new(),
        R::PrsUnmerged(prs) => object!({"prs": prs}),
        R::PrsLinked(prs) => object!({"prs": prs}),
        R::PrNotLinked(pr) => object!({"pr": pr}),
        R::PrAlreadyLinked(pr) => object!({"pr": pr}),
        R::NotDraft(status) => object!({"status": status.as_str()}),
        R::NoEdit => Map::new(),
        R::Superseded { by } => {
            let by: Vec<&str> = by.iter().map(Slug::as_str).collect();
            object!({"by": by})
        }
        R::IssueShared { issue, tasks } => object!({
            "issue": issue,
            "tasks": [tasks[0].as_str(), tasks[1].as_str()],
        }),
        R::IssueIsPr {
            issue,
            task,
            pr_holder,
        } => object!({
            "issue": issue,
            "task": task.as_str(),
            "pr_holder": pr_holder.as_str(),
        }),
        R::OwnPrReferenced { task, pr, change } => object!({
            "task": task.as_str(),
            "pr": pr,
            "change": change,
        }),
        R::Invalid(schema) | R::InvalidPlan(schema) => object!({
            "field": shown(&schema.field),
            "problem": shown(&schema.problem),
        }),
    }
}
