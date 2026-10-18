use super::*;
use crate::domain::project_board::entities::fixtures_tests::{at, person, slug};
use crate::domain::project_board::entities::task::MAX_TITLE_CHARS;
use crate::domain::project_board::value_objects::schema_error::PLAIN_SPACES_ONLY;

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

/// The bytes `discussion`'s message rows take in its file as the codec
/// writes them: `"| "`, the cells joined by `" | "`, `" |"` and a
/// newline; the cells its message number, its author as
/// ``Name (`email`)``, its time and the line, each `|` in a cell written
/// `\|`. What `text_bytes` must never undercount.
fn written(discussion: &DiscussionFields) -> usize {
    let cell = |text: &str| text.replace('|', "\\|");
    let row = |number: usize, message: &Message, line: &str| {
        format!(
            "| {number} | {} (`{}`) | {} | {} |\n",
            cell(&message.author.name),
            cell(&message.author.email),
            message.at.as_str(),
            cell(line)
        )
        .len()
    };
    discussion
        .messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| {
            message
                .text
                .split('\n')
                .map(move |line| row(index + 1, message, line))
        })
        .sum()
}

/// Lines of `|`s (and one `x` to make the bytes even) filling exactly
/// `left` bytes as `text_bytes` counts them, each line taking `per_line`
/// besides its own text.
fn pipe_lines(mut left: usize, per_line: usize) -> Vec<String> {
    let widest = per_line + 2 * MAX_MESSAGE_LINE_CHARS;
    let line = |cost: usize| {
        let rest = cost - per_line;
        format!("{}{}", "|".repeat(rest / 2), "x".repeat(rest % 2))
    };
    let mut lines = vec![];
    while left > widest + per_line + 2 {
        lines.push(line(widest));
        left -= widest;
    }
    if left > widest {
        let cost = left - (per_line + 2);
        lines.push(line(cost));
        left -= cost;
    }
    lines.push(line(left));
    lines
}

/// A discussion's text is bounded by what it takes in its file. The
/// widest case, every character a `|` written `\|` in lines as long as
/// they can be, fills the budget to the byte; its rows as the codec
/// writes them take no more than counted, so with the reserve the whole
/// file fits its cap.
#[test]
fn a_discussions_widest_text_fits_its_file() {
    let mut full = fields();
    full.title = "|".repeat(MAX_TITLE_CHARS);
    full.messages.truncate(1);
    let first = full.messages[0].clone();
    let per_line = identity_bytes(&first.author) + ROW_OVERHEAD_BYTES;
    let lines = pipe_lines(MAX_DISCUSSION_TEXT_BYTES - full.text_bytes(), per_line);
    for chunk in lines.chunks(MAX_MESSAGE_LINES) {
        full.messages.push(Message {
            at: at("09:05:00"),
            text: chunk.join("\n"),
            ..first.clone()
        });
    }
    assert_eq!(
        full.text_bytes(),
        MAX_DISCUSSION_TEXT_BYTES,
        "the budget, to the byte"
    );
    let rows = written(&full);
    assert!(
        rows <= full.text_bytes(),
        "{rows} written > {} counted",
        full.text_bytes()
    );
    assert!(Discussion::new(full.clone()).is_ok(), "exactly the budget");
    let last = full.messages.len() - 1;
    full.messages[last].text.push('x');
    assert_eq!(Discussion::new(full).unwrap_err().field, "text");
}

/// The shortest rows as many as can be: 16,000 one-character lines over
/// 1,024 messages, the three- and four-digit message numbers loaded
/// (messages 1–99 one row each, 1,000–1,024 sixty-four each, the rest in
/// 100–999), fit the file as the codec writes them.
#[test]
fn a_discussions_most_rows_fit_its_file() {
    let mut many = fields();
    let base = many.messages[0].clone();
    let short = (MAX_DISCUSSION_LINES - 99 - 25 * MAX_MESSAGE_LINES) as u32;
    let (each, extra) = (short / 900, short % 900);
    many.messages = (1..=MAX_MESSAGES as u32)
        .map(|number| {
            let lines = match number {
                1..=99 => 1,
                1000.. => MAX_MESSAGE_LINES as u32,
                _ => each + u32::from(number - 100 < extra),
            };
            Message {
                text: vec!["x"; lines as usize].join("\n"),
                ..base.clone()
            }
        })
        .collect();
    assert_eq!(
        (many.messages.len(), many.lines()),
        (MAX_MESSAGES, MAX_DISCUSSION_LINES)
    );
    let rows = written(&many);
    assert!(
        rows <= many.text_bytes(),
        "{rows} written > {} counted",
        many.text_bytes()
    );
    assert!(Discussion::new(many).is_ok());
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

/// A message line spaces its words with plain spaces only: any other space
/// (an em space, a no-break space the normaliser did not see) is refused
/// with the line's field, as no file holds it.
#[test]
fn a_message_line_holds_plain_spaces_only() {
    for space in ['\u{a0}', '\u{2003}', '\u{202f}', '\u{3000}', '\u{205f}'] {
        let mut held = fields();
        held.messages[0].text = format!("Plain\nnot{space}plain");
        let refused = Discussion::new(held).unwrap_err();
        assert_eq!(
            (refused.field.as_str(), refused.problem.as_str()),
            ("messages/0/text/1", PLAIN_SPACES_ONLY),
            "{space:?}"
        );
    }
    let mut held = fields();
    held.messages[0].text = "plain spaces only".into();
    assert!(Discussion::new(held).is_ok());
}
