//! The discussion codec: the example file byte for byte, messages of many
//! lines and an unlinked discussion read back, a message's rows contiguous
//! and alike, and the worst cases byte for byte: the domain's row overhead
//! and file reserve are what the codec writes. Every file is read by the
//! CommonMark/GFM oracle cell by cell.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::discussion::{
    FILE_RESERVE_BYTES, MAX_DISCUSSION_FILE_BYTES, MAX_DISCUSSION_LINES, MAX_DISCUSSION_TEXT_BYTES,
    MAX_MESSAGE_LINE_CHARS, MAX_MESSAGES, ROW_OVERHEAD_BYTES,
};
use crate::domain::project_board::entities::task::MAX_TITLE_CHARS;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::tests::oracle::outline_in;
use crate::infrastructure::persistence::project_board::markdown::{Document, parse};

const EXAMPLE: &str = r#"<!-- quecto-discussion schema=1 -->
# Board CLI: how should list answer?

| Field | Value |
|---|---|
| Id | board-cli-list |
| Task | board-cli |
| State | open |
| Started by | Ada (`ada@example.com`) |
| Created | 2026-10-11T12:00:00Z |
| Updated | 2026-10-11T12:05:00Z |

## Messages
| Message | From | At | Text |
|---|---|---|---|
| 1 | Ada (`ada@example.com`) | 2026-10-11T12:00:00Z | Should `list` answer every task in full? |
| 2 | Rex (`rex@example.com`) | 2026-10-11T12:05:00Z | No: a summary per task. |
| 2 | Rex (`rex@example.com`) | 2026-10-11T12:05:00Z | `show` gives the whole task, `a\|b` and all. |
"#;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

fn message(author: &str, when: &str, text: &str) -> Message {
    Message {
        author: person(author),
        at: at(when),
        text: text.into(),
    }
}

fn example() -> DiscussionFields {
    DiscussionFields {
        id: slug("board-cli-list"),
        title: "Board CLI: how should list answer?".into(),
        task: Some(slug("board-cli")),
        state: DiscussionState::Open,
        started_by: person("Ada"),
        messages: vec![
            message(
                "Ada",
                "2026-10-11T12:00:00Z",
                "Should `list` answer every task in full?",
            ),
            message(
                "Rex",
                "2026-10-11T12:05:00Z",
                "No: a summary per task.\n`show` gives the whole task, `a|b` and all.",
            ),
        ],
        created: at("2026-10-11T12:00:00Z"),
        updated: at("2026-10-11T12:05:00Z"),
    }
}

/// `fields` written and read back as itself, every cell as a reader shows
/// it; the file.
fn round_trips(fields: DiscussionFields) -> String {
    let discussion = Discussion::new(fields).unwrap();
    let file = encode::<DiscussionCodec>(&discussion).unwrap();
    assert_eq!(
        decode::<DiscussionCodec>(file.as_bytes()),
        Ok(discussion),
        "{file}"
    );
    let document: Document = parse(file.as_bytes(), &DiscussionCodec::SHAPE).unwrap();
    assert_eq!(
        outline_in(&file, &document, &DiscussionCodec::SHAPE),
        Ok(())
    );
    file
}

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> String {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1)
}

fn refused_at(file: &str, at: &str) {
    let read = decode::<DiscussionCodec>(file.as_bytes());
    assert!(
        matches!(&read, Err(CodecError::Cell { field, .. }) if *field == at),
        "{read:?}\n{file}"
    );
}

#[test]
fn the_example_discussion_file_is_byte_exact() {
    assert_eq!(round_trips(example()), EXAMPLE);
}

#[test]
fn an_unlinked_closed_discussion_round_trips() {
    let mut fields = example();
    fields.task = None;
    fields.state = DiscussionState::Closed;
    let file = round_trips(fields);
    assert!(!file.contains("| Task |"), "{file}");
    assert!(file.contains("| State | closed |"), "{file}");
}

#[test]
fn a_messages_rows_are_contiguous_and_alike() {
    // Message 1's rows split by message 2's.
    let split = edited(
        "and all. |\n",
        "and all. |\n| 1 | Ada (`ada@example.com`) | 2026-10-11T12:00:00Z | again |\n",
    );
    refused_at(&split, "Messages");
    // A gap in the numbers, or one out of order.
    refused_at(
        &edited(
            "| 2 | Rex (`rex@example.com`) | 2026-10-11T12:05:00Z | No:",
            "| 3 | Rex (`rex@example.com`) | 2026-10-11T12:05:00Z | No:",
        ),
        "Messages",
    );
    // A row of the same message by another author, or at another time.
    refused_at(
        &edited(
            "| 2 | Rex (`rex@example.com`) | 2026-10-11T12:05:00Z | `show`",
            "| 2 | Ada (`ada@example.com`) | 2026-10-11T12:05:00Z | `show`",
        ),
        "Messages",
    );
    refused_at(
        &edited(
            "| 2 | Rex (`rex@example.com`) | 2026-10-11T12:05:00Z | `show`",
            "| 2 | Rex (`rex@example.com`) | 2026-10-11T12:06:00Z | `show`",
        ),
        "Messages",
    );
}

/// The bytes the codec writes besides what the domain's text counts: the
/// marker, the field rows' labels, the headings and the table header.
fn reserve_used(file: &str, fields: &DiscussionFields) -> usize {
    // A row writes its number in at most 4 digits; the domain counts 4.
    let short_numbers: usize = fields
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| (4 - (index + 1).to_string().len()) * message.lines())
        .sum();
    file.len() + short_numbers - fields.text_bytes()
}

/// A discussion of `messages` messages, `lines(message)` lines each, every
/// line `line(message, line)`, by `who`.
fn discussion(
    messages: usize,
    lines: impl Fn(usize) -> usize,
    line: impl Fn(usize, usize) -> String,
    who: &Identity,
    title: String,
) -> DiscussionFields {
    let when = at("9999-12-31T23:59:59Z");
    DiscussionFields {
        id: slug(&format!("{}x", "a".repeat(63))),
        title,
        task: Some(slug(&format!("{}y", "b".repeat(63)))),
        state: DiscussionState::Closed,
        started_by: who.clone(),
        messages: (0..messages)
            .map(|index| Message {
                author: who.clone(),
                at: when.clone(),
                text: (0..lines(index))
                    .map(|row| line(index, row))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect(),
        created: when.clone(),
        updated: when,
    }
}

/// A discussion at the domain's whole budget: every message, the whole
/// text budget spent on `|` (written `\|`), its title, starter and authors
/// all `|` too. One example at the budget, not the largest file the domain
/// admits. The row overhead and reserve the domain counts are what the
/// codec writes, byte for byte, and the file fits its cap.
#[test]
fn the_widest_discussion_fits_byte_for_byte() {
    let who = Identity {
        name: "|".repeat(100),
        email: format!("{}@{}", "|".repeat(126), "|".repeat(127)),
    };
    let title = "|".repeat(MAX_TITLE_CHARS);
    let empty = discussion(
        MAX_MESSAGES,
        |_| 1,
        |_, _| String::new(),
        &who,
        title.clone(),
    );
    // What a row of each message takes with an empty line, and so what the
    // lines may spend: two bytes a `|`, one for a final odd byte.
    let room = MAX_DISCUSSION_TEXT_BYTES - empty.text_bytes();
    let share = |index: usize| room / MAX_MESSAGES + usize::from(index < room % MAX_MESSAGES);
    let line = |index: usize, _: usize| {
        let bytes = share(index);
        format!("{}{}", "|".repeat(bytes / 2), "x".repeat(bytes % 2))
    };
    assert!(share(0) / 2 <= MAX_MESSAGE_LINE_CHARS);
    let widest = discussion(MAX_MESSAGES, |_| 1, line, &who, title);
    assert_eq!(widest.text_bytes(), MAX_DISCUSSION_TEXT_BYTES);
    let file = round_trips(widest.clone());
    let reserve = reserve_used(&file, &widest);
    // Pinned, so a change to what the codec writes is seen. The reserve is
    // small here: the domain counts the title's `|` escaped, as a heading
    // never writes it (256 bytes to spare).
    assert_eq!((file.len(), reserve), (1_043_509, 136));
    assert!(reserve <= FILE_RESERVE_BYTES);
    assert!(file.len() <= MAX_DISCUSSION_FILE_BYTES);
    assert!(file.len() <= DISCUSSION_LIMITS.file_bytes);
    assert!(file.lines().map(str::len).max().unwrap() <= DISCUSSION_LIMITS.line_bytes);
}

/// The most rows: 16,000 lines over every message (numbered 1 to 1024,
/// 1 to 4 digits), each a one-byte line by the shortest author, `A`
/// (`a@b`). The same identity holds: each row is its line, its author and
/// the domain's 43 bytes.
#[test]
fn sixteen_thousand_short_rows_fit_byte_for_byte() {
    let who = Identity {
        name: "A".into(),
        email: "a@b".into(),
    };
    let lines = |index: usize| {
        let base = MAX_DISCUSSION_LINES / MAX_MESSAGES;
        base + usize::from(index < MAX_DISCUSSION_LINES % MAX_MESSAGES)
    };
    let short = discussion(MAX_MESSAGES, lines, |_, _| "x".into(), &who, "T".into());
    assert_eq!(short.lines(), MAX_DISCUSSION_LINES);
    let file = round_trips(short.clone());
    let reserve = reserve_used(&file, &short);
    // Every row the domain counts at 43 bytes past its line and author.
    let rows: usize = file
        .lines()
        .filter(|row| row.starts_with("| ") && row.ends_with(" | x |"))
        .count();
    assert_eq!(rows, MAX_DISCUSSION_LINES);
    assert_eq!(
        (file.len(), file.lines().count(), reserve),
        (751_044, 16_015, 392)
    );
    assert!(reserve <= FILE_RESERVE_BYTES);
    assert!(file.lines().count() <= DISCUSSION_LIMITS.lines);
    // A row of a 4-digit message is the domain's overhead past its text.
    let row = file
        .lines()
        .find(|row| row.starts_with("| 1024 | "))
        .unwrap();
    assert_eq!(
        row.len() + 1,
        ROW_OVERHEAD_BYTES + "x".len() + "A".len() + "a@b".len()
    );
}

/// The longest line: a message line of 4-byte letters at its longest, by
/// the widest author (a name of 4-byte letters, an email of `|` written
/// `\|`), in the last message, numbered 1024.
#[test]
fn the_longest_row_fits_its_line_cap() {
    let who = Identity {
        name: "\u{1d538}".repeat(100),
        email: format!("{}@{}", "|".repeat(126), "|".repeat(127)),
    };
    let longest = discussion(
        MAX_MESSAGES,
        |_| 1,
        |index, _| match index == MAX_MESSAGES - 1 {
            true => "\u{1d538}".repeat(MAX_MESSAGE_LINE_CHARS),
            false => "x".into(),
        },
        &who,
        "T".into(),
    );
    let file = round_trips(longest);
    let line = file.lines().map(str::len).max().unwrap();
    assert!(
        file.lines()
            .any(|row| row.len() == line && row.starts_with("| 1024 | ")),
        "the longest row is message 1024's"
    );
    assert_eq!(line, 4_949);
    assert!(line <= DISCUSSION_LIMITS.line_bytes);
}
