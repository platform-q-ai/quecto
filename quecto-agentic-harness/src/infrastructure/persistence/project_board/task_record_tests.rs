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
fn a_task_too_large_for_its_file_is_not_written() {
    let mut fields = decode(MINIMAL.as_bytes()).unwrap().into_fields();
    fields.description = "𝄞".repeat(65_536);
    let task = Task::new(fields).expect("within the character limit");
    assert!(matches!(encode(&task), Err(RecordError::TooLarge { .. })));
}
