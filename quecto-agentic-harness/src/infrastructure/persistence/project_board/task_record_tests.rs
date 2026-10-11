use super::*;

const FULL: &str = r#"{
  "schema": 1,
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
  "schema": 1,
  "id": "idea",
  "title": "An idea",
  "kind": "chore",
  "status": "draft",
  "created": "2026-10-10T08:00:00Z",
  "updated": "2026-10-10T08:00:00Z"
}
"#;

fn edited(text: &str, from: &str, to: &str) -> Vec<u8> {
    assert!(text.contains(from), "{from:?} is in the file");
    text.replacen(from, to, 1).into_bytes()
}

#[test]
fn a_task_file_reads_and_writes_back_byte_for_byte_with_schema_first() {
    for file in [FULL, MINIMAL] {
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
    let newer = decode(&edited(FULL, "\"schema\": 1", "\"schema\": 2"));
    assert_eq!(newer, Err(RecordError::NewerSchema { found: 2 }));
    assert!(newer.unwrap_err().to_string().ends_with("upgrade quecto"));
    let newer_with_new_keys = edited(FULL, "\"schema\": 1,", "\"schema\": 2,\n  \"labels\": [],");
    assert_eq!(
        decode(&newer_with_new_keys),
        Err(RecordError::NewerSchema { found: 2 })
    );
    assert_eq!(
        decode(&edited(FULL, "\"schema\": 1", "\"schema\": 0")),
        Err(RecordError::UnknownSchema { found: 0 })
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
    assert_eq!(introduced_in(1), Some("0.107.250"));
    assert_eq!(introduced_in(SCHEMA_VERSION + 1), None);
}

#[test]
fn a_newer_schema_names_the_least_quecto_that_can_read_it() {
    let message = RecordError::NewerSchema { found: 2 }.to_string();
    let wanted = [
        "schema 2".to_string(),
        format!("needs a quecto newer than {}", env!("CARGO_PKG_VERSION")),
        "schema 1 arrived in quecto 0.107.250".to_string(),
        "upgrade quecto".to_string(),
    ];
    for part in wanted {
        assert!(message.contains(&part), "{message:?} names {part:?}");
    }
}
