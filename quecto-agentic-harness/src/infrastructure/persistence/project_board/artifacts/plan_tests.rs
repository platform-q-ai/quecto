//! The plan codec: the example file byte for byte, every state, a derived
//! title, the tables' order and items, the team's rows, and the caps.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::claim::{Identity, MAX_EMAIL_BYTES};
use crate::domain::project_board::entities::plan::{MAX_APPROACH_CHARS, MAX_PLAN_TEXT_BYTES};
use crate::domain::project_board::entities::task_parts::{MAX_ITEMS, MAX_PER_ITEM};
use crate::domain::project_board::entities::team::{
    MAX_DEADLINE_SECONDS, MAX_IMAGE_BYTES, MAX_MODEL_BYTES, MAX_ROLES,
};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::schema_error::{PLAIN_SPACES_ONLY, SchemaError};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::MarkdownError;
use crate::infrastructure::persistence::project_board::markdown::parse;
use crate::infrastructure::persistence::project_board::markdown::tests::oracle::outline_in;

const EXAMPLE: &str = r#"<!-- quecto-plan schema=1 -->
# Plan 2 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Version | 2 |
| State | accepted |
| Proposed by | Ada Lovelace (`ada@example.com`) |
| Edited by | Bo (`bo@example.com`) |
| Accepted by | Shane (work) (`shane@example.com`) |
| Accepted at | 2026-10-11T12:00:00Z |
| Deadline seconds | 345600 |
| Image | `ghcr.io/platform-q-ai/quecto_dev:1.2` |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T12:00:00Z |

## Approach
Build the CLI on the store, one op a line.
\# not a heading

- read first
- then write

```sh
quecto board list

quecto board show board-cli
```

## Task items
| Task item | Title |
|---|---|
| store | The store reads and writes `a\|b` |
| cli | The CLI answers one JSON line |
| docs | Docs |

## Task item dependencies
| Task item | Depends on |
|---|---|
| cli | store |
| docs | store |
| docs | cli |

## Criteria
| Task item | Criterion |
|---|---|
| store | Round trips every kind |
| store | Refuses hostile files |
| cli | `quecto board list` answers |

## Roles
| Role | Model | Effort | Members | Turns |
|---|---|---|---|---|
| builder | `gpt-5_codex` | high | 2 | 40 |
| reviewer | `claude-opus/4.5` | max | 1 | - |
"#;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn person(name: &str, email: &str) -> Identity {
    Identity {
        name: name.into(),
        email: email.into(),
    }
}

fn item(id: &str, title: &str, criteria: &[&str], depends_on: &[&str]) -> PlanItem {
    PlanItem {
        id: slug(id),
        title: title.into(),
        criteria: criteria.iter().map(|c| (*c).to_string()).collect(),
        depends_on: depends_on.iter().map(|d| slug(d)).collect(),
    }
}

fn role(name: &str, model: &str, effort: Effort, members: u16, turns: Option<u32>) -> Role {
    Role {
        name: slug(name),
        model: model.into(),
        effort,
        limits: RoleLimits { members, turns },
    }
}

fn example() -> PlanFields {
    PlanFields {
        task: slug("board-cli"),
        version: 2,
        state: PlanState::Accepted,
        approach: "Build the CLI on the store, one op a line.\n# not a heading\n\n\
                   - read first\n- then write\n\n\
                   ```sh\nquecto board list\n\nquecto board show board-cli\n```"
            .into(),
        items: vec![
            item(
                "store",
                "The store reads and writes `a|b`",
                &["Round trips every kind", "Refuses hostile files"],
                &[],
            ),
            item(
                "cli",
                "The CLI answers one JSON line",
                &["`quecto board list` answers"],
                &["store"],
            ),
            item("docs", "Docs", &[], &["store", "cli"]),
        ],
        team: Some(Team {
            roles: vec![
                role("builder", "gpt-5_codex", Effort::High, 2, Some(40)),
                role("reviewer", "claude-opus/4.5", Effort::Max, 1, None),
            ],
            deadline_seconds: Some(345_600),
            image: Some("ghcr.io/platform-q-ai/quecto_dev:1.2".into()),
        }),
        handoff: None,
        proposed_by: person("Ada Lovelace", "ada@example.com"),
        edited_by: Some(person("Bo", "bo@example.com")),
        accepted: Some((
            person("Shane (work)", "shane@example.com"),
            at("2026-10-11T12:00:00Z"),
        )),
        created: at("2026-10-11T09:30:00Z"),
        updated: at("2026-10-11T12:00:00Z"),
    }
}

fn plan(fields: PlanFields) -> Plan {
    Plan::new(fields).unwrap()
}

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> String {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1)
}

/// `file` read by both readers of the CommonMark/GFM oracle, cell by cell,
/// as the document it parses to.
fn read_by_the_oracle(file: &str) {
    let document = parse(file.as_bytes(), &PlanCodec::SHAPE).unwrap();
    assert_eq!(
        outline_in(file, &document, &PlanCodec::SHAPE),
        Ok(()),
        "{file}"
    );
}

#[test]
fn the_example_plan_file_is_byte_exact() {
    let example = plan(example());
    assert_eq!(encode::<PlanCodec>(&example).unwrap(), EXAMPLE);
    assert_eq!(decode::<PlanCodec>(EXAMPLE.as_bytes()), Ok(example));
    read_by_the_oracle(EXAMPLE);
}

#[test]
fn every_plan_state_round_trips() {
    let accepted = example().accepted;
    for state in PlanState::ALL.iter().copied() {
        let accepted = match state {
            PlanState::Accepted | PlanState::Superseded => accepted.clone(),
            PlanState::Draft | PlanState::Discarded => None,
        };
        let bare = PlanFields {
            state,
            accepted: accepted.clone(),
            approach: String::new(),
            items: vec![],
            team: None,
            edited_by: None,
            ..example()
        };
        let lean_team = Team {
            roles: vec![role("solo", "m", Effort::Low, 1, None)],
            deadline_seconds: None,
            image: None,
        };
        let variants = [
            PlanFields {
                state,
                accepted: accepted.clone(),
                ..example()
            },
            bare.clone(),
            PlanFields {
                team: Some(lean_team),
                ..bare
            },
        ];
        for fields in variants {
            let plan = plan(fields);
            let file = encode::<PlanCodec>(&plan).unwrap();
            assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan), "{file}");
            read_by_the_oracle(&file);
        }
    }
}

#[test]
fn a_plan_title_that_is_not_derived_is_refused() {
    for title in ["# Plan 3 of board-cli", "# Plan 2 of other", "# The plan"] {
        let file = edited("# Plan 2 of board-cli", title);
        assert_eq!(
            decode::<PlanCodec>(file.as_bytes()),
            Err(CodecError::NotCanonical),
            "{title}"
        );
    }
}

#[test]
fn criteria_out_of_task_item_order_are_not_canonical() {
    let criteria = "| store | Round trips every kind |\n| store | Refuses hostile files |\n| cli | `quecto board list` answers |\n";
    let dependencies = "| cli | store |\n| docs | store |\n| docs | cli |\n";
    for file in [
        edited(
            criteria,
            "| cli | `quecto board list` answers |\n| store | Round trips every kind |\n| store | Refuses hostile files |\n",
        ),
        edited(
            dependencies,
            "| docs | store |\n| cli | store |\n| docs | cli |\n",
        ),
    ] {
        assert_eq!(
            decode::<PlanCodec>(file.as_bytes()),
            Err(CodecError::NotCanonical),
            "{file}"
        );
    }
}

#[test]
fn a_criterion_of_an_unknown_task_item_is_refused() {
    for (file, table) in [
        (
            edited("| cli | `quecto board list` answers |", "| nope | x |"),
            "Criteria",
        ),
        (
            edited("| docs | cli |", "| nope | cli |"),
            "Task item dependencies",
        ),
    ] {
        let read = decode::<PlanCodec>(file.as_bytes());
        assert!(
            matches!(&read, Err(CodecError::Cell { field, problem }) if *field == table && problem == "nope is no task item of the plan"),
            "{read:?}"
        );
    }
}

#[test]
fn team_rows_without_roles_are_refused() {
    let roles = EXAMPLE.find("\n## Roles").unwrap();
    let without_roles = EXAMPLE[..roles].to_string();
    let read = decode::<PlanCodec>(without_roles.as_bytes());
    assert!(
        matches!(
            &read,
            Err(CodecError::Cell {
                field: "Deadline seconds",
                ..
            })
        ),
        "{read:?}"
    );
    let image_only = without_roles.replacen("| Deadline seconds | 345600 |\n", "", 1);
    let read = decode::<PlanCodec>(image_only.as_bytes());
    assert!(
        matches!(&read, Err(CodecError::Cell { field: "Image", .. })),
        "{read:?}"
    );
    // Without the rows too, it is a plan without a team.
    let no_team = image_only.replacen(
        "| Image | `ghcr.io/platform-q-ai/quecto_dev:1.2` |\n",
        "",
        1,
    );
    let read = decode::<PlanCodec>(no_team.as_bytes()).unwrap();
    assert_eq!(read.fields().team, None);
}

/// A file written before a team's budget was dropped and its deadline
/// became seconds from Start still reads: its `Budget tokens` and
/// absolute `Deadline` rows are read past, with a team or without, so the
/// plan has the default deadline; and it is never written back with them.
#[test]
fn a_legacy_budget_and_deadline_are_read_past_and_never_written() {
    let legacy = edited(
        "| Deadline seconds | 345600 |\n",
        "| Budget tokens | 2000000 |\n| Deadline | 2026-10-18T17:00:00Z |\n",
    );
    let read = decode::<PlanCodec>(legacy.as_bytes()).unwrap();
    let team = read.fields().team.clone().unwrap();
    assert_eq!(team.deadline_seconds, None, "the default deadline");
    assert_eq!(team.roles, example().team.unwrap().roles);
    let written = encode::<PlanCodec>(&read).unwrap();
    assert!(!written.contains("Budget tokens"), "{written}");
    assert!(!written.contains("| Deadline |"), "{written}");
    assert_eq!(written, edited("| Deadline seconds | 345600 |\n", ""));
    // Even unreadable, a legacy row sets nothing, nor holds a plan up.
    let odd = legacy.replacen("| Budget tokens | 2000000 |", "| Budget tokens | lots |", 1);
    assert_eq!(decode::<PlanCodec>(odd.as_bytes()), Ok(read));
    let roles = legacy.find("\n## Roles").unwrap();
    let teamless = legacy[..roles].replacen(
        "| Image | `ghcr.io/platform-q-ai/quecto_dev:1.2` |\n",
        "",
        1,
    );
    let read = decode::<PlanCodec>(teamless.as_bytes()).unwrap();
    assert_eq!(read.fields().team, None);
    // The legacy rows are the only ones the plan does not keep.
    let dropped: Vec<&str> = FIELDS
        .iter()
        .map(|column| column.name)
        .filter(|name| !PlanCodec::keeps(name))
        .collect();
    assert_eq!(dropped, [LEGACY_BUDGET, LEGACY_DEADLINE]);
}

/// A deadline past its bounds is refused as the plan is read.
#[test]
fn a_deadline_out_of_its_bounds_is_refused() {
    for seconds in ["599", "604801", "99999999999"] {
        let file = edited(
            "| Deadline seconds | 345600 |",
            &format!("| Deadline seconds | {seconds} |"),
        );
        let read = decode::<PlanCodec>(file.as_bytes());
        assert!(
            matches!(
                &read,
                Err(CodecError::Cell {
                    field: "Plan" | "Deadline seconds",
                    ..
                })
            ),
            "{seconds}: {read:?}"
        );
    }
}

#[test]
fn turns_dash_reads_as_no_limit() {
    let read = decode::<PlanCodec>(EXAMPLE.as_bytes()).unwrap();
    let roles = &read.fields().team.as_ref().unwrap().roles;
    assert_eq!(roles[0].limits.turns, Some(40));
    assert_eq!(roles[1].limits.turns, None);
    for (turns, field) in [("0", "Plan"), ("--", "Roles"), ("x", "Roles")] {
        let file = edited("| max | 1 | - |", &format!("| max | 1 | {turns} |"));
        let read = decode::<PlanCodec>(file.as_bytes());
        assert!(
            matches!(&read, Err(CodecError::Cell { field: f, .. }) if *f == field),
            "{turns}: {read:?}"
        );
    }
}

/// A slug of 64 bytes, numbered.
fn long_slug(index: usize) -> Slug {
    slug(&format!("{}{index:03}", "a".repeat(61)))
}

/// The widest of every list the domain admits: 128 task items, each with 16
/// criteria and as many dependencies as an acyclic plan has, 16 roles,
/// every optional field, and `approach` and `criterion` as given.
fn widest(approach: String, criterion: impl Fn(usize, usize) -> String) -> PlanFields {
    let items = (0..MAX_ITEMS)
        .map(|index| PlanItem {
            id: long_slug(index),
            title: "|".into(),
            criteria: (0..MAX_PER_ITEM).map(|c| criterion(index, c)).collect(),
            depends_on: (index.saturating_sub(MAX_PER_ITEM)..index)
                .map(long_slug)
                .collect(),
        })
        .collect();
    let name = "\u{1d538}".repeat(100);
    // An email's `|` is written `\|`: the most bytes an email can take.
    // Its domain a `|` too: every byte of the email written twice.
    let email = format!("{}@|", "|".repeat(MAX_EMAIL_BYTES - 2));
    let roles = (0..MAX_ROLES)
        .map(|index| {
            role(
                long_slug(index).as_str(),
                &"m".repeat(MAX_MODEL_BYTES),
                Effort::Medium,
                32,
                Some(u32::MAX),
            )
        })
        .collect();
    let image = format!("{}/{}", "a".repeat(100), "b".repeat(MAX_IMAGE_BYTES - 101));
    PlanFields {
        task: long_slug(999),
        version: 999,
        state: PlanState::Superseded,
        approach,
        items,
        team: Some(Team {
            roles,
            deadline_seconds: Some(MAX_DEADLINE_SECONDS),
            image: Some(image),
        }),
        handoff: None,
        proposed_by: person(&name, &email),
        edited_by: Some(person(&name, &email)),
        accepted: Some((person(&name, &email), at("2026-10-11T12:00:00Z"))),
        created: at("2026-10-11T09:30:00Z"),
        updated: at("2026-10-11T12:00:00Z"),
    }
}

#[test]
fn the_widest_plan_fits_its_limits() {
    // Most lines: the approach all blank lines inside a fence.
    let fenced = format!("```{}```", "\n".repeat(MAX_APPROACH_CHARS - 6));
    let most_lines = widest(fenced, |_, c| format!("c{c}"));
    // Most bytes: the whole text budget spent on `|`, which a cell writes
    // `\|`: each task item's share over its criteria, distinct lengths, the last
    // taking what is left.
    // One byte each for the task items' titles and the roles' models, one for
    // the image (its row is the widest, written as code), and one for an
    // approach of `#`, which brings its section and is written `\#`.
    let budget = MAX_PLAN_TEXT_BYTES - MAX_ITEMS - MAX_ROLES - 1 - 1;
    let lengths = |index: usize, criterion: usize| {
        let share = budget / MAX_ITEMS + usize::from(index < budget % MAX_ITEMS);
        let steps = MAX_PER_ITEM * (MAX_PER_ITEM - 1) / 2;
        let base = (share - steps) / MAX_PER_ITEM;
        let left = share - (base * MAX_PER_ITEM + steps);
        base + criterion + usize::from(criterion == MAX_PER_ITEM - 1) * left
    };
    let mut most_bytes = widest("#".into(), |index, c| "|".repeat(lengths(index, c)));
    // The team's own text cut to the least (one-byte models and image),
    // so the budget goes to cells that write it twice.
    if let Some(team) = most_bytes.team.as_mut() {
        team.roles
            .iter_mut()
            .for_each(|role| role.model = "m".into());
        team.image = Some("a".into());
    }
    // The longest line: the approach one line of 4-byte letters.
    let one_line = "\u{1d538}".repeat(MAX_APPROACH_CHARS);
    let longest_line = widest(one_line, |_, c| format!("c{c}"));
    let mut seen = (0, 0, 0);
    for fields in [most_lines, most_bytes, longest_line] {
        let plan = Plan::new(fields).unwrap();
        let file = encode::<PlanCodec>(&plan).unwrap();
        read_by_the_oracle(&file);
        let lines = file.lines().count();
        let longest = file.lines().map(str::len).max().unwrap();
        seen = (
            seen.0.max(file.len()),
            seen.1.max(lines),
            seen.2.max(longest),
        );
        assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan));
    }
    let (bytes, lines, longest) = seen;
    // The whole budget spent on cells that write it twice: at least
    // 1,077,113 bytes (a lower bound; no search proves it the domain's most;
    // 43 fewer than when a team held a token budget and an absolute
    // deadline, whose rows a deadline in seconds replaced).
    assert!(
        (1_077_113..=PLAN_LIMITS.file_bytes).contains(&bytes),
        "{bytes}"
    );
    // Past the plan's first estimate of 53,000: a fence's blank lines.
    assert!(lines <= PLAN_LIMITS.lines && lines > 53_000, "{lines}");
    // An approach of 65,536 four-byte letters: 262,144 bytes.
    assert!(
        (4 * MAX_APPROACH_CHARS..=PLAN_LIMITS.line_bytes).contains(&longest),
        "{longest}"
    );
}

/// Whether an error names the cap a file is past.
type Over = fn(&CodecError) -> bool;

#[test]
fn one_byte_or_line_over_a_limit_is_refused() {
    let marker = "<!-- quecto-plan schema=1 -->\n";
    // Lines of at most a thousand bytes, so only the size is past a cap.
    let sized = |bytes: usize| {
        let mut file = marker.to_string();
        while file.len() < bytes {
            let line = (bytes - file.len()).min(1_000);
            file.push_str(&format!("{}\n", "x".repeat(line - 1)));
        }
        file
    };
    let lined = |lines: usize| format!("{marker}{}", "x\n".repeat(lines - 1));
    let long_line = |bytes: usize| format!("{marker}# {}\n", "x".repeat(bytes - 2));
    use MarkdownError as E;
    let limits = PLAN_LIMITS;
    let cases: [(String, Over); 3] = [
        (sized(limits.file_bytes + 1), |e| {
            matches!(e, CodecError::Markdown(E::TooLarge { .. }))
        }),
        (lined(limits.lines + 1), |e| {
            matches!(e, CodecError::Markdown(E::TooManyLines { .. }))
        }),
        (long_line(limits.line_bytes + 1), |e| {
            matches!(e, CodecError::Markdown(E::LineTooLong { .. }))
        }),
    ];
    for (file, over) in cases {
        let refused = decode::<PlanCodec>(file.as_bytes()).unwrap_err();
        assert!(over(&refused), "{refused:?}");
    }
    // At each limit the file is refused for what it holds, not its size.
    for file in [
        sized(limits.file_bytes),
        lined(limits.lines),
        long_line(limits.line_bytes),
    ] {
        let refused = decode::<PlanCodec>(file.as_bytes()).unwrap_err();
        let sized = matches!(
            refused,
            CodecError::Markdown(
                E::TooLarge { .. } | E::TooManyLines { .. } | E::LineTooLong { .. }
            )
        );
        assert!(!sized, "{refused:?}");
    }
}

/// Accepted by and Accepted at come together, or neither.
#[test]
fn accepted_by_and_accepted_at_come_together() {
    for row in [
        "| Accepted by | Shane (work) (`shane@example.com`) |\n",
        "| Accepted at | 2026-10-11T12:00:00Z |\n",
    ] {
        let file = edited(row, "");
        let read = decode::<PlanCodec>(file.as_bytes());
        assert!(
            matches!(
                &read,
                Err(CodecError::Cell {
                    field: "Accepted by",
                    ..
                })
            ),
            "{read:?}"
        );
    }
}

/// A dependency on a task item the plan does not have names its table.
#[test]
fn a_dependency_on_an_unknown_task_item_is_refused() {
    let file = edited("| docs | cli |", "| docs | nope |");
    let read = decode::<PlanCodec>(file.as_bytes());
    assert!(
        matches!(
            &read,
            Err(CodecError::Cell {
                field: "Task item dependencies",
                problem,
            }) if problem == "nope is no task item of the plan"
        ),
        "{read:?}"
    );
}

/// An approach the blocks refuse names its field and its line there.
#[test]
fn an_approach_the_blocks_refuse_names_its_field() {
    for (approach, line) in [("abc\n", 2), ("a\n\n\nb", 2), ("- a\n\n- b", 3)] {
        let plan = plan(PlanFields {
            approach: approach.into(),
            ..example()
        });
        let refused = encode::<PlanCodec>(&plan);
        assert!(
            matches!(&refused, Err(CodecError::Cell { field: "Approach", problem }) if problem.starts_with(&format!("line {line}:"))),
            "{approach:?}: {refused:?}"
        );
    }
}

/// A model or image only a code cell holds: an ASCII cell would refuse
/// its underscores or its brackets.
#[test]
fn a_model_and_image_only_a_code_cell_holds_round_trip() {
    for (model, image) in [
        ("a__b_", "localhost/a__b:_x"),
        ("gpt_5", "[::1]:5000/foo:tag"),
    ] {
        let mut fields = example();
        if let Some(team) = fields.team.as_mut() {
            team.roles[0].model = model.into();
            team.image = Some(image.into());
        }
        let plan = plan(fields);
        let file = encode::<PlanCodec>(&plan).unwrap();
        assert!(file.contains(&format!("| Image | `{image}` |\n")), "{file}");
        assert!(file.contains(&format!("| `{model}` |")), "{file}");
        assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan));
        read_by_the_oracle(&file);
    }
}

/// A plan file of the wording before task items (`## Items`) is refused,
/// never read as an alias of `## Task items`.
#[test]
fn an_items_heading_of_the_old_wording_is_refused() {
    let old = edited("## Task items", "## Items");
    let read = decode::<PlanCodec>(old.as_bytes());
    assert_eq!(
        read,
        Err(CodecError::Markdown(MarkdownError::UnknownSection {
            heading: "Items".into()
        })),
        "{read:?}"
    );
}

/// A criterion with a space chat writes between words is held once
/// normalised, for each such space, and round-trips; an em space, which no
/// normaliser makes plain, the domain refuses with its field, never the
/// file at write time.
#[test]
fn a_criterion_with_a_chat_space_is_held_once_normalised() {
    let with_criterion = |criterion: &str| {
        let mut fields = example();
        fields.items[0].criteria[0] = criterion.into();
        Plan::new(fields)
    };
    for space in ['\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}'] {
        let criterion = board_text::line(&format!("{space}Lists answer{space}: fast{space}"));
        assert_eq!(criterion, "Lists answer : fast");
        let held = with_criterion(&criterion).unwrap();
        let file = encode::<PlanCodec>(&held).unwrap();
        assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(held));
        read_by_the_oracle(&file);
    }
    assert_eq!(
        with_criterion("an em\u{2003}space"),
        Err(SchemaError::new("items/0/criteria/0", PLAIN_SPACES_ONLY))
    );
}

/// An approach typed on a French keyboard is held once normalised, and so
/// is a chat space beside a list marker (entry makes it plain: it is never
/// meaningful there); an em space is refused by the domain with the
/// approach's field, never by the file at write time.
#[test]
fn an_approach_with_chat_spaces_is_held_once_normalised() {
    let with_approach = |approach: &str| {
        Plan::new(PlanFields {
            approach: approach.into(),
            ..example()
        })
    };
    let normal = board_text::text("D\u{2019}abord\u{a0}: un spike\u{202f}!");
    assert_eq!(normal, "D\u{2019}abord : un spike !");
    let held = with_approach(&normal).unwrap();
    let file = encode::<PlanCodec>(&held).unwrap();
    assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(held));
    read_by_the_oracle(&file);
    let listed = board_text::text("-\u{a0}first");
    assert_eq!(listed, "- first");
    assert!(with_approach(&listed).is_ok(), "{listed:?}");
    assert_eq!(
        with_approach(&board_text::text("an em\u{2003}space")).map(|_| ()),
        Err(SchemaError::new(
            "approach",
            format!("line 1: {PLAIN_SPACES_ONLY}")
        ))
    );
}

/// #2668: a task item's title and criteria, and the approach (a line
/// opening with them, and a list item), of awkward characters (a
/// backslash before punctuation or at the end, an escaped pipe, a code
/// span holding either) are written, read back as themselves, and shown by
/// both readers of the oracle as written: the codec escapes, never
/// refuses, a backslash.
#[test]
fn awkward_titles_and_criteria_round_trip() {
    let pieces = [
        "x", "\\", "\\\\", "|", "\\|", "\\*", "*", "C:\\temp", "`a\\|b`", "\\`c`", "!", "\\.",
    ];
    let mut checked = 0;
    for first in pieces {
        for second in pieces {
            for third in pieces {
                let text = format!("x {first}{second} {third}");
                let fields = PlanFields {
                    approach: format!("{first}{second} {third} x\n\n- {text}"),
                    items: vec![item("store", &text, &[&text, "plain"], &[])],
                    team: None,
                    ..example()
                };
                let plan = Plan::new(fields).unwrap_or_else(|error| panic!("{text:?}: {error:?}"));
                let file =
                    encode::<PlanCodec>(&plan).unwrap_or_else(|error| panic!("{text:?}: {error}"));
                assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan), "{text:?}");
                read_by_the_oracle(&file);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, pieces.len().pow(3));
}

/// #2668's report: `x \\ y` (two backslashes) as a criterion.
#[test]
fn a_criterion_of_two_backslashes_is_written_escaped() {
    let fields = PlanFields {
        items: vec![item("store", "Store", &["x \\\\ y", "a \\| b"], &[])],
        team: None,
        ..example()
    };
    let plan = Plan::new(fields).unwrap();
    let file = encode::<PlanCodec>(&plan).unwrap();
    // Only a backslash a reader would take for an escape is doubled: the
    // first, before a backslash; the second, before a space, is kept.
    assert!(file.contains("| store | x \\\\\\ y |"), "{file}");
    assert!(file.contains("| store | a \\\\\\| b |"), "{file}");
    assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan));
}
