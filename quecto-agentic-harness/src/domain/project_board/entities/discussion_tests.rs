use super::*;
use crate::domain::project_board::entities::fixtures_tests::{at, person, slug};

/// One edit that must make valid fields invalid.
type Edit = Box<dyn Fn(&mut DiscussionFields)>;

fn message(who: &str, time: &str, text: &str) -> Message {
    Message {
        author: person(who),
        at: at(time),
        text: text.into(),
    }
}

fn fields() -> DiscussionFields {
    DiscussionFields {
        id: slug("store-locking"),
        title: "How the store locks".into(),
        task: None,
        messages: vec![
            message("Ada", "09:00:00", "Should the store take a lease?"),
            message("Bob", "09:05:00", "Yes, with a **TTL**."),
        ],
        closed: None,
        created: at("09:00:00"),
        updated: at("09:10:00"),
    }
}

#[test]
fn a_discussion_builds_open_linked_or_closed() {
    let open = Discussion::new(fields()).unwrap();
    assert_eq!(open.state(), DiscussionState::Open);
    let mut linked = fields();
    linked.task = Some(slug("board-store"));
    assert!(Discussion::new(linked).is_ok());
    let mut closed = fields();
    closed.closed = Some((person("Ada"), at("09:05:00")));
    assert_eq!(
        Discussion::new(closed).unwrap().state(),
        DiscussionState::Closed
    );
    let mut same_second = fields();
    same_second.messages[1].at = at("09:00:00");
    assert!(
        Discussion::new(same_second).is_ok(),
        "two messages in one second"
    );
}

#[test]
fn fields_that_break_a_rule_build_no_discussion() {
    let cases: Vec<(&str, Edit)> = vec![
        ("updated", Box::new(|d| d.updated = at("08:00:00"))),
        ("title", Box::new(|d| d.title = " ".into())),
        ("title", Box::new(|d| d.title = "two\nlines".into())),
        ("messages", Box::new(|d| d.messages.clear())),
        (
            "messages",
            Box::new(|d| {
                let first = d.messages[0].clone();
                d.messages = vec![first; MAX_MESSAGES + 1];
            }),
        ),
        ("messages/0/at", Box::new(|d| d.created = at("08:59:59"))),
        (
            "messages/0/at",
            Box::new(|d| d.messages[0].at = at("09:01:00")),
        ),
        (
            "messages/1/at",
            Box::new(|d| d.messages[1].at = at("08:59:00")),
        ),
        (
            "messages/1/at",
            Box::new(|d| d.messages[1].at = at("09:10:01")),
        ),
        (
            "messages/1/author/email",
            Box::new(|d| d.messages[1].author.email = "bob".into()),
        ),
        (
            "messages/1/text",
            Box::new(|d| d.messages[1].text = " \n".into()),
        ),
        (
            "messages/1/text",
            Box::new(|d| d.messages[1].text = "hidden\u{200b}text".into()),
        ),
        (
            "messages/1/text",
            Box::new(|d| d.messages[1].text = "x".repeat(MAX_MESSAGE_CHARS + 1)),
        ),
        (
            "closed/by/email",
            Box::new(|d| {
                d.closed = Some((
                    Identity {
                        name: "Ada".into(),
                        email: "ada".into(),
                    },
                    at("09:06:00"),
                ))
            }),
        ),
        (
            "closed/at",
            Box::new(|d| d.closed = Some((person("Ada"), at("09:04:59")))),
        ),
        (
            "closed/at",
            Box::new(|d| d.closed = Some((person("Ada"), at("09:10:01")))),
        ),
    ];
    for (field, edit) in cases {
        let mut broken = fields();
        edit(&mut broken);
        assert_eq!(Discussion::new(broken).unwrap_err().field, field);
    }
    let mut longest = fields();
    longest.messages[1].text = "x".repeat(MAX_MESSAGE_CHARS);
    assert!(Discussion::new(longest).is_ok(), "the longest message");
}

/// A discussion's text is bounded: its title, messages and their authors
/// together, exactly to the budget and not a byte past it.
#[test]
fn a_discussions_text_is_bounded_by_its_budget() {
    let mut full = fields();
    full.messages.truncate(1);
    let mut left = MAX_DISCUSSION_TEXT_BYTES - full.text_bytes();
    let author = full.messages[0].author.clone();
    let per_author = author.name.len() + author.email.len();
    while left > 0 {
        let len = (left - per_author).min(MAX_MESSAGE_CHARS);
        full.messages.push(Message {
            author: author.clone(),
            at: at("09:05:00"),
            text: "x".repeat(len),
        });
        left -= len + per_author;
    }
    assert_eq!(full.text_bytes(), MAX_DISCUSSION_TEXT_BYTES);
    assert!(Discussion::new(full.clone()).is_ok(), "exactly the budget");
    let last = full.messages.len() - 1;
    full.messages[last].text.push('x');
    assert_eq!(Discussion::new(full).unwrap_err().field, "text");
}
