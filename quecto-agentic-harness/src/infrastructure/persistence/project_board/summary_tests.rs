use super::{plain, summary};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

fn task(title: &str, description: &str) -> Task {
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse("a").unwrap(),
        title: title.into(),
        kind: TaskKind::Task,
        description: description.into(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        reviews: vec![],
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

#[test]
fn no_board_text_renders_as_markup() {
    let hostile = r#"<img src="https://t.example/p.gif"> ![x](https://t.example/i.png) [link](javascript:x) <script>"#;
    let text = summary(&task(hostile, &format!("{hostile}\n``` still code\n")));
    let title = text.lines().next().unwrap();
    for raw in ["<img", "<script", "![x]", "[link]("] {
        assert!(!title.contains(raw), "{raw} in {title}");
    }
    let body = text.split_once("|\n\n").unwrap().1;
    assert!(
        body.starts_with("````text\n"),
        "the description is a code block longer than its own fences: {body}"
    );
    assert!(body.trim_end().ends_with("````"), "{body}");
}

#[test]
fn plain_escapes_markdown_and_html() {
    assert_eq!(
        plain("a<b>&\"[c](d)!*_`|"),
        r#"a&lt;b&gt;&amp;&quot;\[c\]\(d\)\!\*\_\`\|"#
    );
    assert_eq!(plain("Board store"), "Board store");
}
