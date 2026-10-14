//! A swarm brief as its coordinator reads it (S4c): the task text a
//! started task's coordinator is launched with. Plain labelled lines, so
//! every value is copied into the swarm's calls as it is; within the room
//! the brief's size allowed for its wording ([`SwarmBrief::bytes`]).
use std::fmt::Write;

use super::{SwarmBrief, TEMPLATE_BYTES};

/// The fixed wording, at most a quarter of its room.
const OPENING: &str = "Create this swarm run and its task items exactly as given here, then \
run it as your playbook says. Copy every value as written.";
const ITEMS: &str = "Task items (op=task_item_create), in this order. Give each title \
exactly as written: the board matches each item back by it. For dependencies, pass the \
task_item_id the create of each named item returned. Each item's acceptance lines run \
until its dependencies line.";
const CLOSING: &str = "When the run ends, report its summary to your parent.";
const _: () = assert!(OPENING.len() + ITEMS.len() + CLOSING.len() < TEMPLATE_BYTES / 4);

/// `brief` as its coordinator's task text.
pub fn render(brief: &SwarmBrief) -> String {
    let mut text = String::new();
    // Writing to a String never fails.
    let mut line = |line: std::fmt::Arguments<'_>| {
        text.write_fmt(line).expect("a String takes every write");
        text.push('\n');
    };
    let task = brief.task.as_str();
    let version = brief.plan_version;
    line(format_args!(
        "You coordinate the swarm for board task {task}, plan version {version}. {OPENING}"
    ));
    line(format_args!("\nRun (op=create):"));
    line(format_args!("goal: {}", brief.goal));
    match brief.member_limit {
        Some(limit) => line(format_args!("member_limit: {limit}")),
        None => line(format_args!(
            "member_limit: yours to choose (the plan names no team)"
        )),
    }
    line(format_args!(
        "deadline_in_seconds: {}",
        brief.deadline_in_seconds
    ));
    line(format_args!("criteria:"));
    for criterion in &brief.criteria {
        let (id, kind) = (criterion.id, criterion.kind.as_str());
        line(format_args!(
            "- id {id}, kind {kind}: {}",
            criterion.description
        ));
    }
    for (at, constraint) in brief.constraints.iter().enumerate() {
        line(format_args!(
            "constraint {} (until the next label):",
            at + 1
        ));
        line(format_args!("{constraint}"));
    }
    if let Some(tokens) = brief.usage_budget {
        line(format_args!(
            "\nBudget (op=usage_budget): token_limit {tokens}"
        ));
    }
    line(format_args!("\n{ITEMS}"));
    for item in &brief.items {
        line(format_args!("item {}", item.id.as_str()));
        line(format_args!("title: {}", item.title));
        line(format_args!("acceptance:"));
        for criterion in &item.acceptance {
            line(format_args!("{criterion}"));
        }
        let dependencies: Vec<&str> = item.dependencies.iter().map(|id| id.as_str()).collect();
        match dependencies.as_slice() {
            [] => line(format_args!("dependencies: none")),
            named => line(format_args!("dependencies: {}", named.join(", "))),
        }
    }
    line(format_args!("\n{CLOSING}"));
    assert!(
        text.len() <= brief.bytes(),
        "the rendered brief fits the room its size allowed: {} > {}",
        text.len(),
        brief.bytes()
    );
    text
}

#[cfg(test)]
#[path = "swarm_brief_text_tests.rs"]
mod tests;
