//! A `show` answer as the agent tool gives it (#2481 M4a, review rounds 1
//! and 2): an answer within [`MAX_SHOW_BYTES`] exactly as the board gave
//! it; a longer one cut to fit, its task always whole (as before plans and
//! runs were shown), so a task that takes the room alone leaves the rest
//! counted, not shown. The CLI's answer is never cut: the pocket relay
//! reads it whole.
//!
//! What a cut `show` keeps, in this order, each part with a count of what
//! it left out, the way a listing tells `truncated.more`:
//!
//! 1. the accepted `plan`, whole; if it does not fit (`plan_truncated`),
//!    its fields but the approach and task items, then as much of the
//!    approach as leaves what follows its room (at least half), cut where
//!    no character joins the next, then the leading task items that fit
//!    (`approach_more` characters and `items_more` items left out, and
//!    `approach_next` and `items_next`, where `show` given
//!    `approach_from` and `items_from` reads on);
//! 2. the `runs`, newest first: the latest whole, or with only its leading
//!    task items when it does not fit (`latest_run_items_more`), then
//!    older runs in brief (`runs_more`); none when the latest is left out;
//! 3. the `plans` in brief, newest first (`plans_more`);
//! 4. the files set aside, `quarantined`, in their order
//!    (`quarantined_more`).
//!
//! A page of the plan (`show` given `approach_from` or `items_from`) is
//! cut the same way: its approach, then its items, with their counts and
//! where to read on. Every list stays in its own order.
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::fitted;

/// The most bytes a `show` answer takes in the agent tool, unless its
/// task, always whole (up to its 64 Ki-character description), takes more
/// alone. Twice a listing's cap ([`super::MAX_LISTING_BYTES`]): one plan
/// as people write them (an approach of a few KiB, a few dozen task items)
/// fits whole, with its runs and versions in brief, in about 16 thousand
/// tokens; a plan at the board's limits (320 KiB of text, near 1 MB as
/// JSON) is cut and read on by pages, never the agent's context.
pub const MAX_SHOW_BYTES: usize = 64 * 1024;

/// A character that joins the one before it into one character as read
/// (a combining or enclosing mark, a variation selector, a zero-width
/// joiner, an emoji modifier or tag): a cut never falls before one.
static JOINS_PREVIOUS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{M}\x{200D}\x{1F3FB}-\x{1F3FF}\x{E0020}-\x{E007F}]$").expect("joiners")
});

/// An answered `show`, or a page of its plan, within [`MAX_SHOW_BYTES`]
/// beside its whole task, as the module says.
pub(super) fn capped(answer: Value) -> Value {
    if bytes(&answer) <= MAX_SHOW_BYTES {
        return answer;
    }
    match answer.get("runs").is_some() {
        true => capped_show(answer),
        false => capped_page(answer),
    }
}

/// A whole `show` past the cap, cut as the module says.
fn capped_show(mut answer: Value) -> Value {
    let plan = answer["plan"].take();
    let runs = taken(&mut answer, "runs");
    let plans = taken(&mut answer, "plans");
    let files = taken(&mut answer, "quarantined");
    let (approach_chars, items) = parts_of(&plan);
    let latest_items = runs
        .last()
        .and_then(|run| run["items"].as_array())
        .map_or(0, Vec::len);
    // The markers at their widest: every count at most its whole.
    let markers = json!({
        "plan_truncated": true,
        "approach_more": approach_chars, "approach_next": approach_chars,
        "items_more": items, "items_next": items,
        "latest_run_items_more": latest_items,
        "runs_more": runs.len(),
        "plans_more": plans.len(),
        "quarantined_more": files.len(),
    });
    // What is always shown: the task, the answer's own fields, and room
    // for the markers.
    let used = bytes(&answer) + bytes(&markers);
    let mut room = MAX_SHOW_BYTES.saturating_sub(used);
    // What follows the plan, were it all shown.
    let after = [&runs, &plans, &files]
        .iter()
        .map(|list| list.iter().map(|item| bytes(item) + 1).sum::<usize>())
        .sum();
    answer["plan"] = match plan {
        Value::Null => Value::Null,
        plan if bytes(&plan) <= room => {
            room -= bytes(&plan);
            plan
        }
        plan => {
            answer["plan_truncated"] = json!(true);
            cut_plan(plan, &mut room, after, (0, 0), &mut answer)
        }
    };
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
    within_cap(&answer, used);
    answer
}

/// A page of the plan past the cap: its approach and items cut as a whole
/// `show`'s are, with where to read on.
fn capped_page(mut answer: Value) -> Value {
    let plan = answer["plan"].take();
    let (approach_chars, items) = parts_of(&plan);
    let from = |key: &str| {
        plan[key]
            .as_u64()
            .and_then(|at| usize::try_from(at).ok())
            .unwrap_or(0)
    };
    let at = (from("approach_from"), from("items_from"));
    let markers = json!({
        "approach_more": approach_chars,
        "approach_next": at.0.saturating_add(approach_chars),
        "items_more": items,
        "items_next": at.1.saturating_add(items),
    });
    let used = bytes(&answer) + bytes(&markers);
    let mut room = MAX_SHOW_BYTES.saturating_sub(used);
    answer["plan"] = match plan {
        Value::Null => Value::Null,
        plan => cut_plan(plan, &mut room, 0, at, &mut answer),
    };
    within_cap(&answer, used);
    answer
}

/// The characters of `plan`'s approach and its task items, whichever it
/// holds (none for no plan).
fn parts_of(plan: &Value) -> (usize, usize) {
    let chars = plan["approach"]
        .as_str()
        .map_or(0, |text| text.chars().count());
    (chars, plan["items"].as_array().map_or(0, Vec::len))
}

/// `plan` cut to `room`, leaving `after` bytes (what follows it, at most
/// half the room) for what follows: its other fields whole, the approach
/// cut, then the leading task items that fit; noting on `answer` what was
/// left out of each and, from where its parts began (`from`), where to
/// read on. A plan whose other fields alone do not fit is left out,
/// counted whole, read on from where it began.
fn cut_plan(
    mut plan: Value,
    room: &mut usize,
    after: usize,
    from: (usize, usize),
    answer: &mut Value,
) -> Value {
    let approach = match plan.get_mut("approach").map(Value::take) {
        Some(Value::String(text)) => Some(text),
        Some(other) => unreachable!("a plan's approach is text: {other}"),
        None => None,
    };
    let items = plan
        .get("items")
        .is_some()
        .then(|| taken(&mut plan, "items"));
    if approach.is_some() {
        plan["approach"] = json!("");
    }
    let bare = bytes(&plan);
    let fits = bare <= *room;
    if fits {
        *room -= bare;
    }
    let items_bytes = items.as_ref().map_or(0, |items| {
        items.iter().map(|item| bytes(item) + 1).sum::<usize>()
    });
    if let Some(approach) = approach {
        let shown = match fits {
            true => {
                // What follows keeps its room, up to half; the approach
                // takes the rest.
                let keep = (items_bytes + after).min(*room / 2);
                let prefix = prefix_within(&approach, *room - keep);
                *room -= bytes(&json!(prefix)) - 2;
                prefix
            }
            false => String::new(),
        };
        let (all, kept) = (approach.chars().count(), shown.chars().count());
        if kept < all {
            answer["approach_more"] = json!(all - kept);
            answer["approach_next"] = json!(from.0.saturating_add(kept));
        }
        plan["approach"] = json!(shown);
    }
    if let Some(items) = items {
        let all = items.len();
        let kept = match fits {
            true => fitted(items, room),
            false => vec![],
        };
        if kept.len() < all {
            answer["items_more"] = json!(all - kept.len());
            answer["items_next"] = json!(from.1.saturating_add(kept.len()));
        }
        plan["items"] = Value::Array(kept);
    }
    match fits {
        true => plan,
        false => Value::Null,
    }
}

/// The runs (by number, the latest last and whole) that fit in `room`,
/// newest first, kept in their order; how many were left out, and how
/// many of the latest run's task items. Older runs are shown only beside
/// the latest.
fn fitted_runs(runs: Vec<Value>, room: &mut usize) -> (Vec<Value>, usize, usize) {
    let total = runs.len();
    let mut newest_first = runs.into_iter().rev();
    let mut latest_more = 0;
    let mut shown = Vec::new();
    if let Some(mut latest) = newest_first.next() {
        match bytes(&latest) < *room {
            true => {
                *room -= bytes(&latest) + 1;
                shown.push(latest);
            }
            false => {
                let items = taken(&mut latest, "items");
                if bytes(&latest) < *room {
                    *room -= bytes(&latest) + 1;
                    let all = items.len();
                    let kept = fitted(items, room);
                    latest_more = all - kept.len();
                    latest["items"] = Value::Array(kept);
                    shown.push(latest);
                }
            }
        }
    }
    if !shown.is_empty() {
        shown.extend(fitted(newest_first.collect(), room));
    }
    shown.reverse();
    let more = total - shown.len();
    (shown, more, latest_more)
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

/// The longest leading part of `text` whose JSON string, quotes aside,
/// takes at most `room` bytes, ending where no character joins the next.
fn prefix_within(text: &str, room: usize) -> String {
    let mut taken = 0;
    let mut end = 0;
    for (at, c) in text.char_indices() {
        let size = bytes(&json!(c.to_string())) - 2;
        match taken + size <= room {
            true => {
                taken += size;
                end = at + c.len_utf8();
            }
            false => break,
        }
    }
    while end > 0 && !a_boundary(text, end) {
        end = text[..end].char_indices().last().map_or(0, |(at, _)| at);
    }
    text[..end].to_string()
}

/// Whether `text` may be cut at the byte `at`: the character there joins
/// nothing before it, and the one before is no zero-width joiner.
fn a_boundary(text: &str, at: usize) -> bool {
    let next_joins = text[at..]
        .chars()
        .next()
        .is_some_and(|c| JOINS_PREVIOUS.is_match(c.encode_utf8(&mut [0; 4])));
    !next_joins && !text[..at].ends_with('\u{200D}')
}

/// The bytes `value` takes as JSON.
fn bytes(value: &Value) -> usize {
    value.to_string().len()
}

/// `answer` is within the cap, or holds what is always shown alone.
fn within_cap(answer: &Value, used: usize) {
    assert!(
        bytes(answer) <= MAX_SHOW_BYTES.max(used),
        "a show's answer is within its cap, or its task alone takes more"
    );
}

#[cfg(test)]
#[path = "tool_show_tests.rs"]
mod tests;
