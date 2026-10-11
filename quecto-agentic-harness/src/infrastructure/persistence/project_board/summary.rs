//! `tasks/<id>.md`: a read-only summary written beside each task file in
//! the same commit, for people browsing the branch.
use crate::domain::project_board::entities::task::Task;

/// The summary's markdown. Table cells escape `|`; the description follows
/// as written.
pub fn summary(task: &Task) -> String {
    let task = task.fields();
    let cell = |text: &str| text.replace('|', "\\|");
    let mut rows = vec![
        ("Id", format!("`{}`", task.id.as_str())),
        ("Kind", task.kind.as_str().to_string()),
        ("Status", task.status.as_str().to_string()),
    ];
    if let Some(parent) = &task.parent {
        rows.push(("Parent", format!("`{}`", parent.as_str())));
    }
    if !task.depends_on.is_empty() {
        let names: Vec<String> = task
            .depends_on
            .iter()
            .map(|id| format!("`{}`", id.as_str()))
            .collect();
        rows.push(("Depends on", names.join(", ")));
    }
    if let Some(claim) = &task.claim {
        rows.push((
            "Claimed by",
            format!(
                "{} until {}",
                cell(&claim.holder.name),
                claim.expires.as_str()
            ),
        ));
    }
    if !task.prs.is_empty() {
        let numbers: Vec<String> = task.prs.iter().map(|number| format!("#{number}")).collect();
        rows.push(("Pull requests", numbers.join(", ")));
    }
    rows.push(("Created", task.created.as_str().to_string()));
    rows.push(("Updated", task.updated.as_str().to_string()));
    let table: String = rows
        .iter()
        .map(|(name, value)| format!("| {name} | {value} |\n"))
        .collect();
    let mut text = format!(
        "# {}\n\n> Read-only summary of `tasks/{}.json`, written by quecto. Change the task through quecto's board operations, not by editing this file.\n\n| Field | Value |\n|---|---|\n{table}",
        task.title,
        task.id.as_str()
    );
    if !task.description.is_empty() {
        text.push('\n');
        text.push_str(task.description.trim_end_matches('\n'));
        text.push('\n');
    }
    text
}
