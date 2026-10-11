use super::*;

const FULL: &str = r#"{
  "schema": 3,
  "id": "board-store",
  "title": "Board store",
  "kind": "task",
  "description": "Store **tasks** on a branch.\n\n- one",
  "status": "in_progress",
  "parent": "boards",
  "depends_on": [
    "task-schema"
  ],
  "claim": {
    "holder": {
      "name": "Ada",
      "email": "ada@example.com"
    },
    "since": "2026-10-10T09:00:00Z",
    "expires": "2026-10-10T11:00:00Z"
  },
  "prs": [
    2482,
    2483
  ],
  "created": "2026-10-10T08:00:00Z",
  "updated": "2026-10-10T09:05:00Z"
}
"#;

const MINIMAL: &str = r#"{
  "schema": 3,
  "id": "idea",
  "title": "An idea",
  "kind": "chore",
  "status": "draft",
  "created": "2026-10-10T08:00:00Z",
  "updated": "2026-10-10T08:00:00Z"
}
"#;

const PARTS: &str = r#"{
  "schema": 3,
  "id": "board-store",
  "title": "Board store",
  "kind": "task",
  "status": "in_progress",
  "accepted_plan": 3,
  "claim": {
    "holder": {
      "name": "Ada",
      "email": "ada@example.com"
    },
    "since": "2026-10-10T09:00:00Z",
    "expires": "2026-10-10T11:00:00Z"
  },
  "reviews": [
    {
      "id": "r1-h1",
      "round": 1,
      "reviewer": {
        "name": "Bob",
        "email": "bob@example.com"
      },
      "at": "2026-10-10T10:00:00Z",
      "severity": "high",
      "location": "src/store.rs:42:7",
      "finding": "Lease not checked",
      "verdict": "fixed",
      "proving_test": "stale_lease_is_refused"
    },
    {
      "id": "r1-m1",
      "round": 1,
      "reviewer": {
        "name": "dependabot[bot]",
        "email": "bot@example.com"
      },
      "at": "2026-10-10T10:00:00Z",
      "severity": "medium",
      "finding": "Two **lines**:\n\tthe second indented",
      "verdict": "open"
    }
  ],
  "created": "2026-10-10T08:00:00Z",
  "updated": "2026-10-10T10:00:00Z"
}
"#;

/// An archived task with where it came from and how it ended (schema 3).
const CLOSED: &str = r##"{
  "schema": 3,
  "id": "board-store",
  "title": "Board store",
  "kind": "task",
  "status": "archived",
  "github_issue": 2481,
  "resolution": "superseded",
  "superseded_by": [
    "board-store-v2"
  ],
  "references": [
    "#2470",
    "platform-q-ai/browser-task#18",
    "https://example.com/notes?q=1#top"
  ],
  "created": "2026-10-10T08:00:00Z",
  "updated": "2026-10-10T10:00:00Z"
}
"##;

fn edited(text: &str, from: &str, to: &str) -> Vec<u8> {
    assert!(text.contains(from), "{from:?} is in the file");
    text.replacen(from, to, 1).into_bytes()
}

#[test]
fn a_task_file_reads_and_writes_back_byte_for_byte_with_schema_first() {
    for file in [FULL, MINIMAL, PARTS, CLOSED] {
        let task = decode(file.as_bytes()).expect("valid file");
        assert_eq!(String::from_utf8(encode(&task).unwrap()).unwrap(), file);
    }
}

#[test]
fn a_missing_optional_key_means_not_set() {
    let task = decode(MINIMAL.as_bytes()).unwrap();
    let fields = task.fields();
    assert_eq!(
        (fields.description.as_str(), &fields.parent, &fields.claim),
        ("", &None, &None)
    );
    assert!(fields.depends_on.is_empty() && fields.prs.is_empty());
}

#[test]
fn malformed_files_are_refused_by_the_record() {
    let cases = [
        (
            "unknown key",
            edited(FULL, "\"title\"", "\"extra\": 1,\n  \"title\""),
        ),
        (
            "unknown claim key",
            edited(FULL, "\"since\"", "\"note\": \"x\",\n    \"since\""),
        ),
        (
            "unknown holder key",
            edited(FULL, "\"email\"", "\"login\": \"ada\",\n      \"email\""),
        ),
        (
            "duplicate key",
            edited(FULL, "\"title\"", "\"title\": \"Again\",\n  \"title\""),
        ),
        (
            "missing key",
            edited(MINIMAL, "  \"title\": \"An idea\",\n", ""),
        ),
        ("wrong type", edited(FULL, "2482", "\"2482\"")),
        ("not json", b"{".to_vec()),
    ];
    for (case, bytes) in cases {
        assert!(
            matches!(decode(&bytes), Err(RecordError::Malformed(_))),
            "{case}"
        );
    }
}

#[test]
fn a_newer_schema_asks_for_an_upgrade_and_an_unknown_one_is_refused() {
    let newer = decode(&edited(FULL, "\"schema\": 3", "\"schema\": 4"));
    assert_eq!(newer, Err(RecordError::NewerSchema { found: 4 }));
    assert!(newer.unwrap_err().to_string().ends_with("upgrade quecto"));
    let newer_with_new_keys = edited(FULL, "\"schema\": 3,", "\"schema\": 4,\n  \"labels\": [],");
    assert_eq!(
        decode(&newer_with_new_keys),
        Err(RecordError::NewerSchema { found: 4 })
    );
    assert_eq!(
        decode(&edited(FULL, "\"schema\": 3", "\"schema\": 0")),
        Err(RecordError::UnknownSchema { found: 0 })
    );
    // A schema-1 file is read; one holding a part the task no longer has
    // (its plan, say: an artifact of its own from schema 2) is malformed.
    assert!(decode(&edited(MINIMAL, "\"schema\": 3", "\"schema\": 1")).is_ok());
    assert!(decode(&edited(PARTS, "\"schema\": 3", "\"schema\": 2")).is_ok());
    let old_plan = edited(MINIMAL, "\"schema\": 3,", "\"schema\": 1,\n  \"plan\": [],");
    assert!(matches!(decode(&old_plan), Err(RecordError::Malformed(_))));
    // A key from a later schema is refused in an earlier one's file.
    let early = edited(PARTS, "\"schema\": 3", "\"schema\": 1");
    assert!(
        matches!(decode(&early), Err(RecordError::Malformed(why)) if why.contains("schema 2")),
        "accepted_plan in a schema-1 file"
    );
}

#[test]
fn values_the_domain_refuses_are_named_by_their_path() {
    let cases = [
        ("status", edited(FULL, "\"in_progress\"", "\"wip\"")),
        ("kind", edited(FULL, "\"task\",", "\"story\",")),
        (
            "depends_on/0",
            edited(FULL, "\"task-schema\"", "\"Task Schema\""),
        ),
        ("claim/expires", edited(FULL, "11:00:00Z", "11:00:00+00:00")),
        ("claim", edited(FULL, "11:00:00Z", "11:00:01Z")),
        ("prs/0", edited(FULL, "2482", "0")),
    ];
    for (field, bytes) in cases {
        match decode(&bytes) {
            Err(RecordError::Invalid(error)) => assert_eq!(error.field, field, "{error}"),
            other => panic!("{field}: {other:?}"),
        }
    }
}

#[test]
fn a_file_over_the_size_cap_is_refused_before_it_is_parsed() {
    let mut padded = FULL.as_bytes().to_vec();
    padded.resize(MAX_TASK_FILE_BYTES, b' ');
    assert!(decode(&padded).is_ok(), "exactly the cap is read");
    padded.push(b' ');
    assert_eq!(
        decode(&padded),
        Err(RecordError::TooLarge {
            bytes: MAX_TASK_FILE_BYTES + 1
        })
    );
    let garbage = vec![b'x'; MAX_TASK_FILE_BYTES + 1];
    assert_eq!(
        decode(&garbage),
        Err(RecordError::TooLarge {
            bytes: MAX_TASK_FILE_BYTES + 1
        })
    );
}

#[test]
fn a_task_with_every_field_at_its_cap_is_written_and_read_back() {
    use crate::domain::project_board::entities::claim::{MAX_EMAIL_BYTES, MAX_NAME_CHARS};
    use crate::domain::project_board::entities::task::{
        MAX_DEPENDENCIES, MAX_DESCRIPTION_CHARS, MAX_PRS, MAX_TITLE_CHARS,
    };
    // Four bytes a character is the most any allowed text takes in JSON.
    let wide = |count: usize| "\u{1d11e}".repeat(count);
    let long_slug = |n: usize| Slug::parse(&format!("{n:0>64}")).unwrap();
    let mut fields = decode(FULL.as_bytes()).unwrap().into_fields();
    fields.id = long_slug(0);
    fields.title = wide(MAX_TITLE_CHARS);
    fields.description = wide(MAX_DESCRIPTION_CHARS);
    fields.parent = Some(long_slug(1));
    fields.depends_on = (2..MAX_DEPENDENCIES + 2).map(long_slug).collect();
    fields.prs = (0..MAX_PRS as u64).map(|n| u64::MAX - n).collect();
    let claim = fields.claim.as_mut().unwrap();
    claim.holder.name = wide(MAX_NAME_CHARS);
    claim.holder.email = format!("{}@x.io", "a".repeat(MAX_EMAIL_BYTES - 5));
    let task = Task::new(fields).expect("every field at its cap");
    let bytes = encode(&task).expect("a valid task is always written");
    assert!(bytes.len() <= MAX_TASK_FILE_BYTES);
    assert_eq!(decode(&bytes), Ok(task));
}

#[test]
fn every_schema_names_the_quecto_that_introduced_it() {
    let numbers: Vec<u32> = SCHEMAS.iter().map(|(number, _)| *number).collect();
    assert_eq!(numbers, (1..=SCHEMA_VERSION).collect::<Vec<u32>>());
    assert_eq!(introduced_in(1), Some("0.107.251"));
    assert_eq!(introduced_in(SCHEMA_VERSION + 1), None);
}

#[test]
fn a_newer_schema_names_the_least_quecto_that_can_read_it() {
    let message = RecordError::NewerSchema { found: 4 }.to_string();
    let wanted = [
        "schema 4".to_string(),
        format!("needs a quecto newer than {}", env!("CARGO_PKG_VERSION")),
        "schema 3 arrived in quecto 0.107.287".to_string(),
        "upgrade quecto".to_string(),
    ];
    for part in wanted {
        assert!(message.contains(&part), "{message:?} names {part:?}");
    }
}

#[test]
fn every_nested_record_refuses_unknown_keys() {
    let anchors = ["\"round\"", "\"name\": \"Bob\"", "\"name\": \"Ada\""];
    for anchor in anchors {
        let bytes = edited(PARTS, anchor, &format!("\"extra\": 1, {anchor}"));
        assert!(
            matches!(decode(&bytes), Err(RecordError::Malformed(_))),
            "beside {anchor}"
        );
    }
}

#[test]
fn nested_values_the_domain_refuses_are_named_by_their_path() {
    let cases = [
        (
            "accepted_plan",
            edited(PARTS, "\"accepted_plan\": 3", "\"accepted_plan\": 0"),
        ),
        (
            "reviews/0/severity",
            edited(
                PARTS,
                "\"high\",\n      \"location\"",
                "\"blocker\",\n      \"location\"",
            ),
        ),
        ("reviews/0/verdict", edited(PARTS, "\"fixed\"", "\"done\"")),
        (
            "reviews/0/at",
            edited(PARTS, "\"at\": \"2026-10-10T10:00:00Z\"", "\"at\": \"now\""),
        ),
    ];
    for (field, bytes) in cases {
        match decode(&bytes) {
            Err(RecordError::Invalid(error)) => assert_eq!(error.field, field, "{error}"),
            other => panic!("{field}: {other:?}"),
        }
    }
}

/// Where a task came from and how it ended arrived in schema 3: an older
/// file holding any of it is malformed, and the domain's rules for it are
/// named by their path.
#[test]
fn a_tasks_closure_is_schema_3_and_keeps_the_domains_rules() {
    let task = decode(CLOSED.as_bytes()).unwrap();
    let fields = task.fields();
    assert_eq!(
        (
            fields.github_issue,
            fields.resolution,
            fields.references.len()
        ),
        (Some(2481), Some(Resolution::Superseded), 3)
    );
    for key in ["github_issue", "resolution", "superseded_by", "references"] {
        let only = match key {
            "github_issue" => edited(
                MINIMAL,
                "\"schema\": 3,",
                "\"schema\": 2,\n  \"github_issue\": 7,",
            ),
            "resolution" => edited(
                MINIMAL,
                "\"schema\": 3,",
                "\"schema\": 2,\n  \"resolution\": \"completed\",",
            ),
            "superseded_by" => edited(
                MINIMAL,
                "\"schema\": 3,",
                "\"schema\": 2,\n  \"superseded_by\": [\"other\"],",
            ),
            _ => edited(
                MINIMAL,
                "\"schema\": 3,",
                "\"schema\": 2,\n  \"references\": [\"#1\"],",
            ),
        };
        assert!(
            matches!(decode(&only), Err(RecordError::Malformed(why)) if why.contains("schema 3")),
            "{key} in a schema-2 file"
        );
    }
    let cases = [
        (
            "resolution",
            edited(CLOSED, "\"superseded\"", "\"abandoned\""),
        ),
        ("resolution", edited(CLOSED, "\"archived\"", "\"ready\"")),
        ("resolution", edited(CLOSED, "\"archived\"", "\"done\"")),
        ("github_issue", edited(CLOSED, "2481", "10000000000")),
        ("references/0", edited(CLOSED, "\"#2470\"", "\"#2481\"")),
        ("github_issue", edited(CLOSED, "2481", "0")),
        (
            "superseded_by/0",
            edited(CLOSED, "\"board-store-v2\"", "\"Bad Slug\""),
        ),
        (
            "superseded_by/0",
            edited(CLOSED, "\"board-store-v2\"", "\"board-store\""),
        ),
        ("references/0", edited(CLOSED, "\"#2470\"", "\"#0\"")),
        (
            "references/2",
            edited(
                CLOSED,
                "https://example.com/notes?q=1#top",
                "http://example.com",
            ),
        ),
    ];
    for (field, bytes) in cases {
        let refused = decode(&bytes);
        assert!(
            matches!(&refused, Err(RecordError::Invalid(error)) if error.field == field),
            "{field}: {refused:?}"
        );
    }
}

/// The schema gate judges values, as it does for `accepted_plan`: a
/// schema-3 key holding no value (`null` in a single value, an empty list)
/// in an older file reads as not set, as every optional key does; a list
/// set to `null` is malformed, as every list is.
#[test]
fn a_schema_3_key_holding_no_value_reads_as_not_set_in_any_schema() {
    for empty in [
        "\"github_issue\": null,",
        "\"resolution\": null,",
        "\"superseded_by\": [],",
        "\"references\": [],",
    ] {
        let older = edited(
            MINIMAL,
            "\"schema\": 3,",
            &format!("\"schema\": 2,\n  {empty}"),
        );
        let task = decode(&older);
        assert!(task.is_ok(), "{empty}: {task:?}");
    }
    for list in ["superseded_by", "references"] {
        let null = edited(
            MINIMAL,
            "\"schema\": 3,",
            &format!("\"schema\": 3,\n  \"{list}\": null,"),
        );
        assert!(
            matches!(decode(&null), Err(RecordError::Malformed(_))),
            "{list}"
        );
    }
}
