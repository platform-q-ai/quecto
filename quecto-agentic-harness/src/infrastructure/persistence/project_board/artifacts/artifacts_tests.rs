//! The artifact codec over a toy kind (test-only): encode and decode, the
//! schema read from the marker alone, the cell helpers, and a value that
//! does not write its document again refused.
use super::cells::{self, ABSENT};
use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::persistence::project_board::markdown::{
    Body, BodyShape, CellKind, Column, Limits, MarkdownError, Section, SectionShape, Shape,
    blocks_of, text_of,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Toy {
    title: String,
    id: Slug,
    status: TaskStatus,
    pr: Option<u64>,
    holder: Identity,
    branch: String,
    note: String,
    times: Vec<(u64, Timestamp)>,
}

const FIELDS: &[Column<'static>] = &[
    cells::column("Id", CellKind::Ascii),
    cells::column("Status", CellKind::Ascii),
    cells::column("Pr", CellKind::Ascii),
    cells::column("Holder", CellKind::Text),
    cells::column("Branch", CellKind::Code),
];

const TIMES: &[Column<'static>] = &[
    cells::column("Round", CellKind::Ascii),
    cells::column("At", CellKind::Ascii),
];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: "Note",
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: "Times",
        body: BodyShape::Table(TIMES),
    },
];

/// The toy kind's codec.
struct ToyCodec;

impl ArtifactCodec for ToyCodec {
    type Value = Toy;
    const KIND: &'static str = "toy";
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "toy",
        fields: FIELDS,
        sections: SECTIONS,
        limits: Limits {
            file_bytes: 1 << 16,
            lines: 1_024,
            line_bytes: 1_024,
        },
    };

    fn to_document(toy: &Toy) -> Result<Document, CodecError> {
        let mut sections = vec![];
        let note = blocks_of(&toy.note)?;
        if !note.is_empty() {
            sections.push(Section {
                heading: "Note".into(),
                body: Body::Prose(note),
            });
        }
        if !toy.times.is_empty() {
            let rows = toy
                .times
                .iter()
                .map(|(round, at)| vec![cells::number(*round), cells::time(at)]);
            sections.push(Section {
                heading: "Times".into(),
                body: Body::Table(rows.collect()),
            });
        }
        Ok(Document {
            schema: Self::SCHEMA,
            title: toy.title.clone(),
            fields: vec![
                ("Id".into(), cells::slug(&toy.id)),
                ("Status".into(), cells::word(toy.status.as_str())),
                (
                    "Pr".into(),
                    cells::optional(
                        "Pr",
                        toy.pr.map(|pr| cells::reference("Pr", pr)).transpose()?,
                    )?,
                ),
                ("Holder".into(), cells::identity("Holder", &toy.holder)?),
                ("Branch".into(), toy.branch.clone()),
            ],
            sections,
        })
    }

    fn from_document(document: &Document) -> Result<Toy, CodecError> {
        let fields = cells::Fields::of(document);
        let field = |name: &'static str| fields.required(name);
        let pr = match cells::read_optional(field("Pr")?) {
            Some(cell) => Some(cells::read_reference("Pr", cell)?),
            None => None,
        };
        let (mut note, mut times) = (String::new(), vec![]);
        for section in &document.sections {
            match &section.body {
                Body::Prose(blocks) => note = text_of(blocks),
                Body::Table(rows) => {
                    for row in rows {
                        times.push((
                            cells::read_number("Round", &row[0])?,
                            cells::read_time("At", &row[1])?,
                        ));
                    }
                }
            }
        }
        Ok(Toy {
            title: document.title.clone(),
            id: cells::read_slug("Id", field("Id")?)?,
            status: cells::read_word("Status", field("Status")?, TaskStatus::parse)?,
            pr,
            holder: cells::read_identity("Holder", field("Holder")?)?,
            branch: field("Branch")?.to_string(),
            note,
            times,
        })
    }
}

/// A codec of the same kind whose value forgets the title's case, so
/// its value writes another document than it read.
struct Lowercased;

impl ArtifactCodec for Lowercased {
    type Value = Toy;
    const KIND: &'static str = ToyCodec::KIND;
    const SCHEMA: u32 = ToyCodec::SCHEMA;
    const SHAPE: Shape<'static> = ToyCodec::SHAPE;

    fn to_document(toy: &Toy) -> Result<Document, CodecError> {
        ToyCodec::to_document(toy)
    }

    fn from_document(document: &Document) -> Result<Toy, CodecError> {
        let toy = ToyCodec::from_document(document)?;
        Ok(Toy {
            title: toy.title.to_lowercase(),
            ..toy
        })
    }
}

fn person(name: &str, email: &str) -> Identity {
    Identity {
        name: name.into(),
        email: email.into(),
    }
}

fn toy() -> Toy {
    Toy {
        title: "A toy".into(),
        id: Slug::parse("a-toy").unwrap(),
        status: TaskStatus::InProgress,
        pr: Some(12),
        holder: person("Shane (work)", "shane@example.com"),
        branch: "feat/a<b>".into(),
        note: "# not a heading\n\n- one\n- two\n\n```sh\nx\n\ny\n```".into(),
        times: vec![(1, Timestamp::parse("2026-10-11T09:30:00Z").unwrap())],
    }
}

#[test]
fn a_toy_round_trips_through_its_file() {
    for toy in [
        toy(),
        Toy {
            pr: None,
            note: String::new(),
            times: vec![],
            ..toy()
        },
    ] {
        let file = encode::<ToyCodec>(&toy).unwrap();
        assert_eq!(decode::<ToyCodec>(file.as_bytes()), Ok(toy), "{file}");
    }
    let file = encode::<ToyCodec>(&toy()).unwrap();
    assert!(
        file.contains("| Status | in_progress |\n| Pr | #12 |\n"),
        "{file}"
    );
    assert!(file.contains("| Branch | `feat/a<b>` |\n"), "{file}");
    let none = encode::<ToyCodec>(&Toy { pr: None, ..toy() }).unwrap();
    assert!(none.contains(&format!("| Pr | {ABSENT} |\n")), "{none}");
}

#[test]
fn the_schema_is_read_from_the_marker_alone() {
    let file = encode::<ToyCodec>(&toy()).unwrap();
    let newer = file.replacen("schema=1", "schema=2", 1).replacen(
        "| Branch |",
        "| Reviewer | ada |\n| Branch |",
        1,
    );
    assert_eq!(
        decode::<ToyCodec>(newer.as_bytes()),
        Err(CodecError::NewerSchema { found: 2, known: 1 })
    );
    // At this schema the same field is unknown.
    let unknown = newer.replacen("schema=2", "schema=1", 1);
    assert!(matches!(
        decode::<ToyCodec>(unknown.as_bytes()),
        Err(CodecError::Markdown(MarkdownError::UnknownField { .. }))
    ));
    let older = file.replacen("schema=1", "schema=0", 1);
    assert_eq!(
        decode::<ToyCodec>(older.as_bytes()),
        Err(CodecError::UnknownSchema { found: 0 })
    );
    for unmarked in [
        file.replacen("quecto-toy", "quecto-task", 1),
        String::new(),
        "x\n".into(),
    ] {
        assert_eq!(
            decode::<ToyCodec>(unmarked.as_bytes()),
            Err(CodecError::Schema),
            "{unmarked}"
        );
    }
}

#[test]
fn an_identity_cell_reads_back_name_and_email() {
    for holder in [
        person("Shane (work)", "shane@example.com"),
        person("Ada", "odd)@example.com)"),
        person("dependabot[bot]", "bot@example.com"),
        person("A (b) (c", "a(b)@x"),
        // In a code span a reader shows the email as it is: no math, no
        // link, a pipe escaped for the table alone.
        person("Ada", "$x$@b"),
        person("Ada", "a|b@x"),
        person("A (b", "mailto:a.b+c_d@x"),
    ] {
        let cell = cells::identity("Holder", &holder).unwrap();
        assert_eq!(cell, format!("{} (`{}`)", holder.name, holder.email));
        assert_eq!(cells::read_identity("Holder", &cell), Ok(holder.clone()));
        let toy = Toy { holder, ..toy() };
        let file = encode::<ToyCodec>(&toy).unwrap();
        assert_eq!(decode::<ToyCodec>(file.as_bytes()), Ok(toy));
    }
    for unread in [
        "Ada",
        "Ada ()",
        "Ada (``)",
        "Ada (`a@b)",
        "Ada (a@b`)",
        "Ada (a@b)",
        "(`a@b`)",
        "Ada (`a b@c`)",
        " Ada (`a@b`)",
        "Ada (`nobody`)",
    ] {
        assert!(
            matches!(
                cells::read_identity("Holder", unread),
                Err(CodecError::Cell {
                    field: "Holder",
                    ..
                })
            ),
            "{unread:?}"
        );
    }
    // Not an identity, or an email no code span holds.
    for refused in [person("Ada <x>", "a@b"), person("Ada", "a`b@x")] {
        assert!(
            matches!(
                cells::identity("Holder", &refused),
                Err(CodecError::Cell {
                    field: "Holder",
                    ..
                })
            ),
            "{refused:?}"
        );
    }
}

#[test]
fn an_identity_the_inline_rules_refuse_is_refused() {
    for holder in [
        person("Ada &amp; Bo", "a@b"),
        person("Ada `x", "a@b"),
        person("[x](http://y.example)", "a@b"),
        person("ada@example.com", "a@b"),
        person("Ada mailto:x", "a@b"),
        person("A (`", "a@b"),
        person("$x", "a@b"),
    ] {
        // Refused by the helper, naming the field, before any render.
        assert!(
            matches!(
                cells::identity("Holder", &holder),
                Err(CodecError::Cell {
                    field: "Holder",
                    ..
                })
            ),
            "{holder:?}"
        );
        let toy = Toy {
            holder: holder.clone(),
            ..toy()
        };
        assert!(
            matches!(
                encode::<ToyCodec>(&toy),
                Err(CodecError::Cell {
                    field: "Holder",
                    ..
                })
            ),
            "{holder:?}"
        );
    }
}

#[test]
fn a_value_that_rerenders_differently_is_not_canonical() {
    let file = encode::<ToyCodec>(&toy()).unwrap();
    assert_eq!(
        decode::<Lowercased>(file.as_bytes()),
        Err(CodecError::NotCanonical)
    );
    let lower = encode::<ToyCodec>(&Toy {
        title: "a toy".into(),
        ..toy()
    })
    .unwrap();
    assert!(decode::<Lowercased>(lower.as_bytes()).is_ok());
}

#[test]
fn the_cells_read_only_their_own_spelling() {
    assert_eq!(cells::read_number("N", "0"), Ok(0));
    assert_eq!(
        cells::read_number("N", "18446744073709551615"),
        Ok(u64::MAX)
    );
    for unread in ["", "01", "+1", "-1", "1.0", "18446744073709551616", "x"] {
        assert!(cells::read_number("N", unread).is_err(), "{unread:?}");
    }
    assert_eq!(cells::read_reference("Pr", "#7"), Ok(7));
    for unread in ["7", "#0", "#07", "##7", "# 7"] {
        assert!(cells::read_reference("Pr", unread).is_err(), "{unread:?}");
    }
    assert!(cells::read_slug("Id", "Not-A-Slug").is_err());
    assert!(cells::read_time("At", "2026-10-11 09:30:00").is_err());
    assert!(cells::read_word("Status", "inprogress", TaskStatus::parse).is_err());
    assert_eq!(cells::read_optional(ABSENT), None);
    assert_eq!(cells::read_optional("#1"), Some("#1"));
}

#[test]
fn a_prose_text_the_blocks_refuse_is_refused_when_encoded() {
    for note in ["a\n", "- a\n\n- b", "a\tb"] {
        let toy = Toy {
            note: note.into(),
            ..toy()
        };
        assert!(
            matches!(encode::<ToyCodec>(&toy), Err(CodecError::Markdown(_))),
            "{note:?}"
        );
    }
}

/// `encode` takes a value only if the file it writes gives that value
/// back: what a cell cannot spell, or a value its codec reads otherwise,
/// is refused when written, never stored to be refused when read.
#[test]
fn a_value_its_file_would_not_give_back_is_refused_when_encoded() {
    assert_eq!(encode::<Lowercased>(&toy()), Err(CodecError::NotCanonical));
    let unnumbered = Toy {
        pr: Some(0),
        ..toy()
    };
    assert!(matches!(
        encode::<ToyCodec>(&unnumbered),
        Err(CodecError::Cell { field: "Pr", .. })
    ));
    assert!(matches!(
        cells::reference("Pr", 0),
        Err(CodecError::Cell { field: "Pr", .. })
    ));
    assert_eq!(cells::reference("Pr", 7), Ok("#7".to_string()));
    assert!(matches!(
        cells::optional("Pr", Some(ABSENT.to_string())),
        Err(CodecError::Cell { field: "Pr", .. })
    ));
    assert_eq!(cells::optional("Pr", None), Ok(ABSENT.to_string()));
}

/// A codec whose kind is not its shape's writes and reads nothing.
#[test]
fn a_codec_whose_kind_is_not_its_shapes_is_refused() {
    struct Elsewhere;
    impl ArtifactCodec for Elsewhere {
        type Value = Toy;
        const KIND: &'static str = "other";
        const SCHEMA: u32 = ToyCodec::SCHEMA;
        const SHAPE: Shape<'static> = ToyCodec::SHAPE;
        fn to_document(toy: &Toy) -> Result<Document, CodecError> {
            ToyCodec::to_document(toy)
        }
        fn from_document(document: &Document) -> Result<Toy, CodecError> {
            ToyCodec::from_document(document)
        }
    }
    assert_eq!(ToyCodec::KIND, ToyCodec::SHAPE.kind);
    let file = encode::<ToyCodec>(&toy()).unwrap();
    assert_eq!(encode::<Elsewhere>(&toy()), Err(CodecError::Schema));
    assert_eq!(
        decode::<Elsewhere>(file.as_bytes()),
        Err(CodecError::Schema)
    );
}

/// A schema past what a u32 holds was still written by something newer.
#[test]
fn a_schema_past_u32_reads_as_newer() {
    let file = encode::<ToyCodec>(&toy()).unwrap();
    let huge = file.replacen("schema=1", "schema=4294967296", 1);
    assert_eq!(
        decode::<ToyCodec>(huge.as_bytes()),
        Err(CodecError::NewerSchema {
            found: 4_294_967_296,
            known: 1
        })
    );
}

/// An email is held in a code span: it may hold `|` and `\`, never a
/// backtick (the code cell's allowlist).
#[test]
fn an_email_a_code_span_holds_round_trips() {
    for email in ["a|b@x", "a\\b@x", "a\\|b@x"] {
        let toy = Toy {
            holder: person("Ada", email),
            ..toy()
        };
        let file = encode::<ToyCodec>(&toy).unwrap();
        assert_eq!(decode::<ToyCodec>(file.as_bytes()), Ok(toy), "{file}");
    }
}
