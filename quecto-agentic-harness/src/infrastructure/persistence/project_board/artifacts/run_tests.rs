//! The run codec: the example file byte for byte, a running run, the
//! fold-back's tables in item order, and the widest run inside the caps.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::run::MAX_RUN_TEXT_BYTES;
use crate::domain::project_board::entities::task_parts::{
    MAX_ITEMS, MAX_LINE_CHARS, MAX_OWNER_CHARS, MAX_PER_ITEM,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::MarkdownError;
use crate::infrastructure::persistence::project_board::markdown::parse;
use crate::infrastructure::persistence::project_board::markdown::tests::oracle::outline_in;

const EXAMPLE: &str = r#"<!-- quecto-run schema=1 -->
# Run 3 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Number | 3 |
| Plan version | 2 |
| State | succeeded |
| Started | 2026-10-11T12:05:00Z |
| Ended | 2026-10-11T14:40:00Z |
| Cost tokens | 1840000 |

## Items
| Item | State |
|---|---|
| store | done |
| cli | done |
| docs | blocked |

## Owners
| Item | Owner |
|---|---|
| store | W1 builder |
| cli | W2 builder |

## Evidence
| Item | Evidence |
|---|---|
| store | [CI run](https://github.com/platform-q-ai/quecto/actions/runs/1) |
| store | 26 tests pass, `a\|b` included |
| cli | `quecto board list` answers in one line |
"#;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn item(id: &str, state: ItemState, owner: Option<&str>, evidence: &[&str]) -> RunItem {
    RunItem {
        id: slug(id),
        state,
        owner: owner.map(str::to_string),
        evidence: evidence.iter().map(|line| (*line).to_string()).collect(),
    }
}

fn example() -> RunFields {
    RunFields {
        task: slug("board-cli"),
        number: 3,
        plan_version: 2,
        started: at("2026-10-11T12:05:00Z"),
        ended: Some(at("2026-10-11T14:40:00Z")),
        outcome: Some(RunOutcome::Succeeded),
        cost: Some(1_840_000),
        items: vec![
            item(
                "store",
                ItemState::Done,
                Some("W1 builder"),
                &[
                    "[CI run](https://github.com/platform-q-ai/quecto/actions/runs/1)",
                    "26 tests pass, `a|b` included",
                ],
            ),
            item(
                "cli",
                ItemState::Done,
                Some("W2 builder"),
                &["`quecto board list` answers in one line"],
            ),
            item("docs", ItemState::Blocked, None, &[]),
        ],
    }
}

fn running() -> RunFields {
    RunFields {
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
        ..example()
    }
}

fn round_trips(fields: RunFields) -> String {
    let run = Run::new(fields).unwrap();
    let file = encode::<RunCodec>(&run).unwrap();
    assert_eq!(decode::<RunCodec>(file.as_bytes()), Ok(run), "{file}");
    read_by_the_oracle(&file);
    file
}

/// `file` read by both readers of the CommonMark/GFM oracle, cell by cell,
/// as the document it parses to.
fn read_by_the_oracle(file: &str) {
    let document = parse(file.as_bytes(), &RunCodec::SHAPE).unwrap();
    assert_eq!(
        outline_in(file, &document, &RunCodec::SHAPE),
        Ok(()),
        "{file}"
    );
}

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> String {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1)
}

fn refused_at(file: &str, at: &str) {
    let read = decode::<RunCodec>(file.as_bytes());
    assert!(
        matches!(&read, Err(CodecError::Cell { field, .. }) if *field == at),
        "{read:?}\n{file}"
    );
}

#[test]
fn the_example_run_file_is_byte_exact() {
    assert_eq!(round_trips(example()), EXAMPLE);
}

#[test]
fn a_running_run_has_no_end_cost_or_items() {
    let file = round_trips(running());
    assert_eq!(
        file,
        "<!-- quecto-run schema=1 -->\n# Run 3 of board-cli\n\n| Field | Value |\n|---|---|\n\
         | Task | board-cli |\n| Number | 3 |\n| Plan version | 2 |\n| State | running |\n\
         | Started | 2026-10-11T12:05:00Z |\n"
    );
    // The domain refuses a cost or items before the fold-back.
    refused_at(&format!("{file}| Cost tokens | 5 |\n"), "Run");
    let items = format!("{file}\n## Items\n| Item | State |\n|---|---|\n| store | done |\n");
    refused_at(&items, "Run");
    for outcome in RunOutcome::ALL {
        let ended = RunFields {
            outcome: Some(*outcome),
            ended: Some(at("2026-10-11T12:05:00Z")),
            ..running()
        };
        round_trips(ended);
    }
}

#[test]
fn running_with_an_end_is_refused() {
    let running = round_trips(running());
    refused_at(
        &format!("{running}| Ended | 2026-10-11T14:40:00Z |\n"),
        "Ended",
    );
    refused_at(&edited("| Ended | 2026-10-11T14:40:00Z |\n", ""), "Ended");
    refused_at(
        &edited("| State | succeeded |", "| State | done |"),
        "State",
    );
}

#[test]
fn owners_and_evidence_round_trip() {
    let mut fields = example();
    fields.items[0].owner = None;
    fields.items[1].evidence.clear();
    round_trips(fields.clone());
    for item in &mut fields.items {
        item.owner = None;
        item.evidence.clear();
    }
    let bare = round_trips(fields);
    assert!(
        !bare.contains("## Owners") && !bare.contains("## Evidence"),
        "{bare}"
    );
    // Out of item order, or of an item the run does not report.
    let swapped = edited(
        "| store | W1 builder |\n| cli | W2 builder |",
        "| cli | W2 builder |\n| store | W1 builder |",
    );
    assert_eq!(
        decode::<RunCodec>(swapped.as_bytes()),
        Err(CodecError::NotCanonical)
    );
    refused_at(
        &edited("| cli | W2 builder |", "| nope | W2 builder |"),
        "Owners",
    );
    refused_at(
        &edited("| cli | W2 builder |", "| store | W2 builder |"),
        "Owners",
    );
    refused_at(
        &edited(
            "| cli | `quecto board list`",
            "| nope | `quecto board list`",
        ),
        "Evidence",
    );
}

#[test]
fn evidence_with_a_bare_url_is_refused() {
    for text in [
        "see https://x.example/log",
        "log at http://x.example",
        "[log](http://x.example/log)",
    ] {
        let mut fields = example();
        fields.items[2].evidence = vec![text.into()];
        let run = Run::new(fields.clone()).unwrap();
        assert!(
            matches!(
                encode::<RunCodec>(&run),
                Err(CodecError::Markdown(MarkdownError::Link { .. }))
            ),
            "{text}"
        );
        fields.items[2].evidence.clear();
        fields.items[2].owner = Some(text.into());
        let run = Run::new(fields).unwrap();
        assert!(
            matches!(
                encode::<RunCodec>(&run),
                Err(CodecError::Markdown(MarkdownError::Link { .. }))
            ),
            "{text}"
        );
    }
}

/// A slug of 64 bytes, numbered.
fn long_slug(index: usize) -> Slug {
    slug(&format!("{}{index:03}", "a".repeat(61)))
}

/// The run the domain admits at its widest: every item, owner and line of
/// evidence, the text budget spent as `evidence` gives it.
fn widest(evidence: impl Fn(usize, usize) -> String) -> RunFields {
    let items = (0..MAX_ITEMS)
        .map(|index| RunItem {
            id: long_slug(index),
            state: ItemState::Blocked,
            owner: Some("|".repeat(MAX_OWNER_CHARS)),
            evidence: (0..MAX_PER_ITEM)
                .map(|line| evidence(index, line))
                .collect(),
        })
        .collect();
    RunFields {
        task: long_slug(999),
        number: 999,
        plan_version: 999,
        started: at("2026-10-11T12:05:00Z"),
        ended: Some(at("9999-12-31T23:59:59Z")),
        outcome: Some(RunOutcome::Succeeded),
        cost: Some((1 << 53) - 1),
        items,
    }
}

#[test]
fn the_widest_run_fits() {
    // Most bytes and lines: the whole text budget spent on `|`, written
    // `\|`: each item's share over its evidence, distinct lengths, the last
    // line taking what is left.
    let budget = MAX_RUN_TEXT_BYTES - MAX_ITEMS * MAX_OWNER_CHARS;
    let lengths = |index: usize, line: usize| {
        let share = budget / MAX_ITEMS + usize::from(index < budget % MAX_ITEMS);
        let steps = MAX_PER_ITEM * (MAX_PER_ITEM - 1) / 2;
        let base = (share - steps) / MAX_PER_ITEM;
        let left = share - (base * MAX_PER_ITEM + steps);
        base + line + usize::from(line == MAX_PER_ITEM - 1) * left
    };
    let most = widest(|index, line| "|".repeat(lengths(index, line)));
    // The longest line: evidence of 4-byte letters.
    let longest_line = widest(|index, line| match (index, line) {
        (0, 0) => "\u{1d538}".repeat(MAX_LINE_CHARS),
        _ => format!("e{line}"),
    });
    let mut seen = (0, 0, 0);
    for fields in [most, longest_line] {
        let file = round_trips(fields);
        let longest = file.lines().map(str::len).max().unwrap();
        seen = (
            seen.0.max(file.len()),
            seen.1.max(file.lines().count()),
            seen.2.max(longest),
        );
    }
    let (bytes, lines, longest) = seen;
    // The whole budget spent: 691,567 bytes.
    assert!(
        (691_000..=RUN_LIMITS.file_bytes).contains(&bytes),
        "{bytes}"
    );
    assert!(lines <= RUN_LIMITS.lines && lines > 2_300, "{lines}");
    assert!(
        longest <= RUN_LIMITS.line_bytes && longest > 2_000,
        "{longest}"
    );
}

/// No outcome is spelt `running`: the State cell tells a running run from
/// an ended one by that word alone.
#[test]
fn no_outcome_is_spelt_running() {
    assert!(
        RunOutcome::ALL
            .iter()
            .all(|outcome| outcome.as_str() != RUNNING)
    );
}
