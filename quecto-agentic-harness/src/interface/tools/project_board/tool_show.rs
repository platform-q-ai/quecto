//! A `show` answer as the agent tool gives it (#2481 M4a, review round
//! 1): within [`MAX_SHOW_BYTES`], its task always whole (as before plans
//! and runs were shown), so a task that takes the room alone leaves the
//! rest counted, not shown. The CLI's answer is never cut: the pocket
//! relay reads it whole.
//!
//! What fits is kept in this order, each part with a count of what it
//! left out, the way a listing tells `truncated.more`:
//!
//! 1. the accepted `plan`, whole; if it does not fit, its fields but the
//!    approach and task items, then as much of the approach as half the
//!    room left takes, then the leading task items that fit
//!    (`plan_truncated`: `approach_more` characters and `items_more`
//!    items left out);
//! 2. the `runs`, newest first: the latest whole, or with only its
//!    leading task items when it does not fit (`latest_run_items_more`),
//!    then older runs in brief (`runs_more`);
//! 3. the `plans` in brief, newest first (`plans_more`);
//! 4. the files set aside, `quarantined`, in their order
//!    (`quarantined_more`).
//!
//! Every list stays in its own order (plans by version, runs by number).
use serde_json::{Value, json};

use super::fitted;

/// The most bytes a `show` answer takes in the agent tool, unless its
/// task, always whole (up to its 64 Ki-character description), takes more
/// alone. Twice a listing's cap ([`super::MAX_LISTING_BYTES`]):
/// one plan as people write them (an approach of a few KiB, a few dozen
/// task items) fits whole, with its runs and versions in brief, in about
/// 16 thousand tokens; a plan at the board's limits (320 KiB of text,
/// near 1 MB as JSON) is cut, never the agent's context.
pub const MAX_SHOW_BYTES: usize = 64 * 1024;

/// An answered `show`, its plan, runs, plan versions and set-aside files
/// fitted within [`MAX_SHOW_BYTES`] beside its whole task, as the module
/// says.
pub(super) fn capped(mut answer: Value) -> Value {
    let plan = answer["plan"].take();
    let runs = taken(&mut answer, "runs");
    let plans = taken(&mut answer, "plans");
    let files = taken(&mut answer, "quarantined");
    // The markers at their widest: every count at most its whole.
    let (approach_chars, items) = match &plan {
        Value::Object(plan) => (
            plan["approach"]
                .as_str()
                .map_or(0, |text| text.chars().count()),
            plan["items"].as_array().map_or(0, Vec::len),
        ),
        _ => (0, 0),
    };
    let latest_items = runs
        .last()
        .and_then(|run| run["items"].as_array())
        .map_or(0, Vec::len);
    let markers = json!({
        "plan_truncated": {"approach_more": approach_chars, "items_more": items},
        "latest_run_items_more": latest_items,
        "runs_more": runs.len(),
        "plans_more": plans.len(),
        "quarantined_more": files.len(),
    });
    // What is always shown: the task, the answer's own fields, and room
    // for the markers.
    let used = answer.to_string().len() + markers.to_string().len();
    let mut room = MAX_SHOW_BYTES.saturating_sub(used);
    answer["plan"] = fitted_plan(plan, &mut room, &mut answer);
    let (runs, more, latest_more) = fitted_runs(runs, &mut room);
    answer["runs"] = Value::Array(runs);
    noted(&mut answer, "latest_run_items_more", latest_more);
    noted(&mut answer, "runs_more", more);
    let total = plans.len();
    let mut shown = fitted(plans.into_iter().rev().collect(), &mut room);
    shown.reverse();
    noted(&mut answer, "plans_more", total - shown.len());
    answer["plans"] = Value::Array(shown);
    let total = files.len();
    let shown = fitted(files, &mut room);
    noted(&mut answer, "quarantined_more", total - shown.len());
    answer["quarantined"] = Value::Array(shown);
    assert!(
        answer.to_string().len() <= MAX_SHOW_BYTES.max(used),
        "a show's answer is within its cap, or its task alone takes more"
    );
    answer
}

/// The array `answer` holds at `key`, taken out of it (an empty array
/// left in its place, so the room it takes is counted).
fn taken(answer: &mut Value, key: &str) -> Vec<Value> {
    match std::mem::replace(&mut answer[key], json!([])) {
        Value::Array(items) => items,
        other => unreachable!("a show's {key} is an array: {other}"),
    }
}

/// `answer` notes `count` under `key`, when something was left out.
fn noted(answer: &mut Value, key: &str, count: usize) {
    if count > 0 {
        answer[key] = json!(count);
    }
}

/// `plan` whole when it fits in `room`; otherwise cut as the module says,
/// noting `plan_truncated` on `answer`; taking its bytes from `room`.
fn fitted_plan(mut plan: Value, room: &mut usize, answer: &mut Value) -> Value {
    // `null` is counted already: the answer held it while room was told.
    if plan.is_null() {
        return plan;
    }
    let whole = plan.to_string().len();
    if whole <= *room {
        *room -= whole;
        return plan;
    }
    let approach = match plan["approach"].take() {
        Value::String(text) => text,
        other => unreachable!("a plan's approach is text: {other}"),
    };
    let items = taken(&mut plan, "items");
    let (approach_chars, item_count) = (approach.chars().count(), items.len());
    plan["approach"] = json!("");
    let bare = plan.to_string().len();
    let (plan, shown_chars, shown_items) = match bare <= *room {
        true => {
            *room -= bare;
            let prefix = prefix_within(&approach, *room / 2);
            *room -= json_bytes(&prefix) - 2;
            let shown_chars = prefix.chars().count();
            plan["approach"] = json!(prefix);
            let items = fitted(items, room);
            let shown_items = items.len();
            plan["items"] = Value::Array(items);
            (plan, shown_chars, shown_items)
        }
        // Its fields alone do not fit (a team at the board's limits beside
        // a huge task): the plan is left out, counted whole.
        false => (Value::Null, 0, 0),
    };
    answer["plan_truncated"] = json!({
        "approach_more": approach_chars - shown_chars,
        "items_more": item_count - shown_items,
    });
    plan
}

/// The runs (by number, the latest last and whole) that fit in `room`,
/// newest first, kept in their order; how many older runs were left out,
/// and how many of the latest run's task items.
fn fitted_runs(runs: Vec<Value>, room: &mut usize) -> (Vec<Value>, usize, usize) {
    let total = runs.len();
    let mut newest_first: Vec<Value> = runs.into_iter().rev().collect();
    let mut shown = Vec::new();
    let mut latest_more = 0;
    if !newest_first.is_empty() {
        let mut latest = newest_first.remove(0);
        let whole = json_bytes(&latest) + 1;
        if whole <= *room {
            *room -= whole;
            shown.push(latest);
        } else {
            let items = taken(&mut latest, "items");
            let bare = json_bytes(&latest) + 1;
            if bare <= *room {
                *room -= bare;
                let all = items.len();
                let kept = fitted(items, room);
                latest_more = all - kept.len();
                latest["items"] = Value::Array(kept);
                shown.push(latest);
            }
        }
    }
    if shown.len() == 1 {
        shown.extend(fitted(newest_first, room));
    }
    shown.reverse();
    let more = total - shown.len();
    (shown, more, latest_more)
}

/// The longest leading part of `text` whose JSON string, quotes aside,
/// takes at most `bytes`.
fn prefix_within(text: &str, bytes: usize) -> String {
    let mut taken = 0;
    let mut prefix = String::new();
    for c in text.chars() {
        let size = json_bytes(&c.to_string()) - 2;
        match taken + size <= bytes {
            true => {
                taken += size;
                prefix.push(c);
            }
            false => break,
        }
    }
    prefix
}

/// The bytes `value` takes as JSON.
fn json_bytes(value: &impl serde::Serialize) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |json| json.len())
}

#[cfg(test)]
#[path = "tool_show_tests.rs"]
mod tests;
