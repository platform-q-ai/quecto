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

const PARTS: &str = r#"{
  "schema": 1,
  "id": "board-store",
  "title": "Board store",
  "kind": "task",
  "status": "in_progress",
  "plan": [
    {
      "title": "Spike",
      "criteria": [
        "CAS proved"
      ]
    }
  ],
  "items": [
    {
      "id": "plumbing",
      "title": "Plumbing",
      "state": "done",
      "owner": "worker-1",
      "evidence": [
        "log"
      ]
    },
    {
      "id": "cas",
      "title": "CAS",
      "criteria": [
        "stale lease refused"
      ],
      "depends_on": [
        "plumbing"
      ],
      "state": "todo"
    }
  ],
  "team": {
    "roles": [
      {
        "name": "worker",
        "model": "gpt-5.5",
        "effort": "high",
        "limits": {
          "members": 2,
          "turns": 200
        }
      }
    ],
    "budget": {
      "tokens": 5000000
    },
    "deadline": "2026-10-11T00:00:00Z",
    "image": "ghcr.io/platform-q-ai/quecto:0.107"
  },
  "claim": {
    "holder": {
      "name": "Ada",
      "email": "ada@example.com"
    },
    "since": "2026-10-10T09:00:00Z",
    "expires": "2026-10-10T11:00:00Z"
  },
  "runs": [
    {
      "id": "r1",
      "started": "2026-10-10T09:01:00Z",
      "ended": "2026-10-10T09:30:00Z",
      "outcome": "lost"
    },
    {
      "id": "r2",
      "started": "2026-10-10T09:31:00Z"
    }
  ],
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
    }
  ],
  "created": "2026-10-10T08:00:00Z",
  "updated": "2026-10-10T10:00:00Z"
}
"#;

fn edited(text: &str, from: &str, to: &str) -> Vec<u8> {
    assert!(text.contains(from), "{from:?} is in the file");
    text.replacen(from, to, 1).into_bytes()
}

#[test]
fn a_task_file_reads_and_writes_back_byte_for_byte_with_schema_first() {
    for file in [FULL, MINIMAL, PARTS] {
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

#[test]
fn every_nested_record_refuses_unknown_keys() {
    let anchors = [
        "\"title\": \"Spike\"",
        "\"title\": \"Plumbing\"",
        "\"roles\"",
        "\"model\"",
        "\"members\"",
        "\"tokens\"",
        "\"started\": \"2026-10-10T09:31:00Z\"",
        "\"round\"",
        "\"name\": \"Bob\"",
    ];
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
        ("items/0/state", edited(PARTS, "\"done\"", "\"finished\"")),
        (
            "items/1/depends_on/0",
            edited(PARTS, "\"plumbing\"\n      ]", "\"Plumbing\"\n      ]"),
        ),
        (
            "team/roles/0/effort",
            edited(
                PARTS,
                "\"high\",\n        \"limits\"",
                "\"xhigh\",\n        \"limits\"",
            ),
        ),
        (
            "team/deadline",
            edited(PARTS, "2026-10-11T00:00:00Z", "tomorrow"),
        ),
        ("runs/0/outcome", edited(PARTS, "\"lost\"", "\"crashed\"")),
        (
            "runs/1",
            edited(
                PARTS,
                "\"started\": \"2026-10-10T09:31:00Z\"",
                "\"started\": \"2026-10-10T09:31:00Z\", \"outcome\": \"failed\"",
            ),
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
