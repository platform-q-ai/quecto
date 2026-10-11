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
        state: DiscussionState::Open,
        started_by: person("Ada"),
        messages: vec![
            message("Ada", "09:00:00", "Should the store take a lease?"),
            message("Bob", "09:05:00", "Yes, with a **TTL**.\nAnd a renewal."),
        ],
        created: at("09:00:00"),
        updated: at("09:10:00"),
    }
}

/// `count` lines of `x`.
fn lines(count: usize) -> String {
    vec!["x"; count].join("\n")
}

#[test]
fn a_discussion_builds_open_linked_or_closed() {
    assert!(Discussion::new(fields()).is_ok());
    let mut linked = fields();
    linked.task = Some(slug("board-store"));
    assert!(Discussion::new(linked).is_ok());
    let mut closed = fields();
    closed.state = DiscussionState::Closed;
    assert!(Discussion::new(closed).is_ok());
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
        (
            "started_by/email",
            Box::new(|d| d.started_by.email = "ada".into()),
        ),
        ("messages", Box::new(|d| d.messages.clear())),
        (
            "messages",
            Box::new(|d| {
                let first = d.messages[0].clone();
                d.messages = vec![first; MAX_MESSAGES + 1];
            }),
        ),
        (
            "messages",
            Box::new(|d| {
                let mut long = d.messages[1].clone();
                long.text = lines(MAX_MESSAGE_LINES);
                let past = MAX_DISCUSSION_LINES / MAX_MESSAGE_LINES;
                d.messages.truncate(1);
                d.messages.extend(std::iter::repeat_n(long, past));
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
            "messages/1/text/0",
            Box::new(|d| d.messages[1].text = " ".into()),
        ),
        (
            "messages/1/text/1",
            Box::new(|d| d.messages[1].text = "a\n\nb".into()),
        ),
        (
            "messages/1/text/1",
            Box::new(|d| d.messages[1].text = "a\n".into()),
        ),
        (
            "messages/1/text/0",
            Box::new(|d| d.messages[1].text = "hidden\u{200b}text".into()),
        ),
        (
            "messages/1/text/0",
            Box::new(|d| d.messages[1].text = "x".repeat(MAX_MESSAGE_LINE_CHARS + 1)),
        ),
        (
            "messages/1/text",
            Box::new(|d| d.messages[1].text = lines(MAX_MESSAGE_LINES + 1)),
        ),
    ];
    for (field, edit) in cases {
        let mut broken = fields();
        edit(&mut broken);
        assert_eq!(Discussion::new(broken).unwrap_err().field, field);
    }
}

/// The widest message, and the most lines a discussion holds.
#[test]
fn a_discussion_holds_its_widest_message_and_its_most_lines() {
    let mut longest = fields();
    let widest = "x".repeat(MAX_MESSAGE_LINE_CHARS);
    longest.messages[1].text = vec![widest.as_str(); MAX_MESSAGE_LINES].join("\n");
    assert!(Discussion::new(longest).is_ok(), "the longest message");
    let mut most = fields();
    let mut long = most.messages[1].clone();
    long.text = lines(MAX_MESSAGE_LINES);
    let fill = (MAX_DISCUSSION_LINES - 1) / MAX_MESSAGE_LINES;
    let left = MAX_DISCUSSION_LINES - 1 - fill * MAX_MESSAGE_LINES;
    most.messages.truncate(1);
    most.messages
        .extend(std::iter::repeat_n(long.clone(), fill));
    long.text = lines(left);
    most.messages.push(long);
    let total: usize = most
        .messages
        .iter()
        .map(|m| m.text.split('\n').count())
        .sum();
    assert_eq!(total, MAX_DISCUSSION_LINES);
    assert!(
        Discussion::new(most.clone()).is_ok(),
        "exactly the most lines"
    );
    let last = most.messages.len() - 1;
    most.messages[last].text.push_str("\nx");
    assert_eq!(Discussion::new(most).unwrap_err().field, "messages");
}

/// A discussion's text is bounded: its title, who started it, and its
/// message lines each with its author, exactly to the budget and not a
/// byte past it.
#[test]
fn a_discussions_text_is_bounded_by_its_budget() {
    let mut full = fields();
    full.messages.truncate(1);
    let mut left = MAX_DISCUSSION_TEXT_BYTES - full.text_bytes();
    let author = full.messages[0].author.clone();
    let per_line = author.name.len() + author.email.len();
    // Single-line messages, the widest that leave room for one more.
    while left > 0 {
        let room = left - per_line;
        let len = match room > MAX_MESSAGE_LINE_CHARS {
            true if room - MAX_MESSAGE_LINE_CHARS <= per_line => {
                MAX_MESSAGE_LINE_CHARS - per_line - 1
            }
            true => MAX_MESSAGE_LINE_CHARS,
            false => room,
        };
        full.messages.push(Message {
            author: author.clone(),
            at: at("09:05:00"),
            text: "x".repeat(len),
        });
        left -= len + per_line;
    }
    assert_eq!(full.text_bytes(), MAX_DISCUSSION_TEXT_BYTES);
    assert!(Discussion::new(full.clone()).is_ok(), "exactly the budget");
    let last = full.messages.len() - 1;
    full.messages[last].text.push('x');
    assert_eq!(Discussion::new(full).unwrap_err().field, "text");
}
