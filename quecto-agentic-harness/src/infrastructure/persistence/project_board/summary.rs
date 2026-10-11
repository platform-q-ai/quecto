//! `tasks/<id>.md`: a read-only summary written beside each task file in
//! the same commit, for people browsing the branch on GitHub. A board's
//! text is anyone's to write, so nothing in it renders as markup: titles
//! and names have every markdown and HTML character escaped (no raw HTML,
//! links or tracking images), and the description is shown as a code
//! block.
use crate::domain::project_board::entities::task::Task;

/// Text with every character markdown or HTML gives meaning escaped.
pub fn plain(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
            | '!' | '|' | '~' | '^' => {
                escaped.push('\\');
                escaped.push(c);
            }
            c => escaped.push(c),
        }
    }
    escaped
}

/// `text` as a fenced code block, its fence longer than any run of
/// backticks inside it.
fn code_block(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{}\n{fence}\n", text.trim_end_matches('\n'))
}

/// The summary's markdown.
pub fn summary(task: &Task) -> String {
    let task = task.fields();
    let id = |slug: &crate::domain::project_board::value_objects::slug::Slug| {
        format!("`{}`", slug.as_str())
    };
    let mut rows = vec![
        ("Id", id(&task.id)),
        ("Kind", task.kind.as_str().to_string()),
        ("Status", task.status.as_str().to_string()),
    ];
    if let Some(parent) = &task.parent {
        rows.push(("Parent", id(parent)));
    }
    if !task.depends_on.is_empty() {
        let names: Vec<String> = task.depends_on.iter().map(id).collect();
        rows.push(("Depends on", names.join(", ")));
    }
    if let Some(claim) = &task.claim {
        let holder = plain(&claim.holder.name);
        rows.push((
            "Claimed by",
            format!("{holder} until {}", claim.expires.as_str()),
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
        plain(&task.title),
        task.id.as_str()
    );
    if !task.description.is_empty() {
        text.push('\n');
        text.push_str(&code_block(&task.description));
    }
    text
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
