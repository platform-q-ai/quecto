use super::*;
use crate::domain::project_board::entities::fixtures_tests::{at, person, slug};
use crate::domain::project_board::entities::task::MAX_TITLE_CHARS;

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
            Box::new(|d| d.messages[1].text = "line\u{2028}separator".into()),
        ),
        (
            "messages/1/text/0",
            Box::new(|d| d.messages[1].text = "paragraph\u{2029}separator".into()),
        ),
        (
            "messages/1/text/0",
            Box::new(|d| d.messages[1].text = "next\u{85}line".into()),
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

/// The bytes one message row of `author` writing `line` takes in its
/// file, as the codec writes it (`|` escaped), with the widest message
/// number: what `text_bytes` must never undercount.
fn row(author: &Identity, at: &Timestamp, line: &str) -> usize {
    let cell = |text: &str| text.replace('|', "\\|");
    format!(
        "| {MAX_MESSAGES} | {} ({}) | {} | {} |\n",
        cell(&author.name),
        cell(&author.email),
        at.as_str(),
        cell(line)
    )
    .len()
}

/// A discussion's text is bounded by what it takes in its file: the
/// worst case, every character a `|` written `\|` and every row as short
/// and as many as can be, fills the budget exactly, and its rows as the
/// codec writes them fit inside the budget, so the whole file fits its
/// cap with the reserve.
#[test]
fn a_discussions_text_is_bounded_by_what_its_file_takes() {
    let mut full = fields();
    full.title = "|".repeat(MAX_TITLE_CHARS);
    full.messages.truncate(1);
    let author = full.messages[0].author.clone();
    let wide = vec!["|".repeat(MAX_MESSAGE_LINE_CHARS); MAX_MESSAGE_LINES].join("\n");
    let per_wide = full.messages[0].clone();
    loop {
        let mut next = full.clone();
        next.messages.push(Message {
            text: wide.clone(),
            at: at("09:05:00"),
            ..per_wide.clone()
        });
        match next.text_bytes() <= MAX_DISCUSSION_TEXT_BYTES {
            true => full = next,
            false => break,
        }
    }
    // The rest in one message of `|`s, to the byte.
    let per_line = identity_bytes(&author) + ROW_OVERHEAD_BYTES;
    let mut left = MAX_DISCUSSION_TEXT_BYTES - full.text_bytes() - per_line;
    let mut lines = vec![];
    while left > 0 {
        let take = (left / 2).min(MAX_MESSAGE_LINE_CHARS);
        assert!(take > 0, "an even remainder: {left}");
        lines.push("|".repeat(take));
        left -= 2 * take;
        if left > 0 {
            left -= per_line;
        }
    }
    full.messages.push(Message {
        author: author.clone(),
        at: at("09:05:00"),
        text: lines.join("\n"),
    });
    assert_eq!(
        full.text_bytes(),
        MAX_DISCUSSION_TEXT_BYTES,
        "the budget, to the byte"
    );
    let rows: usize = full
        .messages
        .iter()
        .flat_map(|message| {
            message
                .text
                .split('\n')
                .map(|line| row(&message.author, &message.at, line))
        })
        .sum();
    assert!(
        rows <= full.text_bytes(),
        "{rows} written > {} counted",
        full.text_bytes()
    );
    const { assert!(MAX_DISCUSSION_TEXT_BYTES + FILE_RESERVE_BYTES <= MAX_DISCUSSION_FILE_BYTES) };
    assert!(Discussion::new(full.clone()).is_ok(), "exactly the budget");
    let last = full.messages.len() - 1;
    full.messages[last].text.push('x');
    assert_eq!(Discussion::new(full).unwrap_err().field, "text");
}

fn identity_bytes(who: &Identity) -> usize {
    let escaped = |text: &str| text.len() + text.matches('|').count();
    escaped(&who.name) + escaped(&who.email)
}

/// The text counted is the title, who started it, and every message line
/// as its row writes it: the line, its author and the row's fixed bytes,
/// each `|` twice, and no `\n` between lines.
#[test]
fn a_discussions_text_counts_each_row_as_written() {
    let discussion = fields();
    let ada = "Ada".len() + "ada@example.com".len();
    let bob = "Bob".len() + "bob@example.com".len();
    let row = ROW_OVERHEAD_BYTES;
    let expected = "How the store locks".len()
        + ada
        + ("Should the store take a lease?".len() + ada + row)
        + ("Yes, with a **TTL**.".len() + bob + row)
        + ("And a renewal.".len() + bob + row);
    assert_eq!(discussion.text_bytes(), expected);
    let mut piped = fields();
    piped.title = "a|b".into();
    assert_eq!(
        piped.text_bytes(),
        expected - "How the store locks".len() + 4,
        "a | counts twice"
    );
}
