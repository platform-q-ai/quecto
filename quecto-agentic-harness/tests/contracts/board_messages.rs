//! `BoardMessages` on the SQLite adapter (#2275, #2276): a recipient's
//! unread messages are counted by recipient and status as Python counts
//! them, and a message is inserted `accepted`, answering its id; `send`'s
//! messages read back as `dict(row)`, and the inbox binds its flag as
//! Python's `sqlite3` does.
use quecto::application::swarm::dto::{BoardLocation, MessageRow, NewMessage};
use quecto::application::swarm::ports::BoardRepository;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

#[test]
fn unread_messages_are_counted_and_a_message_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "INSERT INTO messages(sender,recipient,body,status) VALUES('a','worker','x','accepted');
             INSERT INTO messages(sender,recipient,body,status) VALUES('a','worker','x','consumed');
             INSERT INTO messages(sender,recipient,body,status) VALUES('a','other','x','accepted');
             INSERT INTO messages(sender,recipient,body,status) VALUES('a','5','x','accepted');",
        )
        .unwrap();
    let mut seen = Vec::new();
    repository
        .atomic(false, &mut |transaction| {
            seen.push(transaction.inbox_count(&json!("worker"))?);
            seen.push(transaction.inbox_count(&json!(5))?);
            seen.push(transaction.inbox_count(&json!(null))?);
            seen.push(transaction.insert_message("parent", &json!("worker"), "hello")?);
            seen.push(transaction.inbox_count(&json!("worker"))?);
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, [1, 1, 0, 5, 2], "TEXT affinity finds '5' by 5");
    let row: (String, String, String, String) = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT sender,recipient,body,status FROM messages WHERE id=5",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "parent".to_owned(),
            "worker".to_owned(),
            "hello".to_owned(),
            "accepted".to_owned()
        )
    );
}

fn repository(dir: &tempfile::TempDir) -> (SqliteBoardRepository, std::path::PathBuf) {
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    (repository, database)
}

fn sent(
    recipient: serde_json::Value,
    revision: Option<&str>,
    supersedes: Option<i64>,
) -> NewMessage {
    NewMessage {
        sender: "worker".to_owned(),
        recipient,
        body: "hello".to_owned(),
        revision: revision.map(str::to_owned),
        supersedes: supersedes.map(serde_json::Value::from),
    }
}

/// #2276: `send`'s insert names the revision and what it supersedes, a
/// message reads back as `dict(row)` in table order (the migrated columns
/// last), found by an id Python binds (`"1"` finds 1), and addressed only
/// to its own recipient (a recipient `5` is stored as the text `'5'`).
#[test]
fn a_sent_message_reads_back_as_dict_row() {
    let dir = tempfile::tempdir().unwrap();
    let (repository, _) = repository(&dir);
    let mut seen = Vec::new();
    repository
        .atomic(false, &mut |transaction| {
            let first = transaction.send_message(&sent(json!("parent"), None, None))?;
            let second = transaction.send_message(&sent(json!(5), Some("abc1"), Some(first)))?;
            transaction.set_message_status(&json!(first), "superseded")?;
            transaction.set_superseded_by(&json!(first), second)?;
            seen.push(
                transaction
                    .message(&json!("1"))?
                    .map(MessageRow::into_value),
            );
            seen.push(transaction.message(&json!(2))?.map(MessageRow::into_value));
            seen.push(transaction.message(&json!(3))?.map(MessageRow::into_value));
            seen.push(
                transaction
                    .addressed_message(&json!(2), "5")?
                    .map(MessageRow::into_value),
            );
            seen.push(
                transaction
                    .addressed_message(&json!(2), "parent")?
                    .map(MessageRow::into_value),
            );
            Ok(())
        })
        .unwrap();
    let row = |id, recipient: &str, status: &str, revision, supersedes, by| {
        Some(json!({
            "id": id, "sender": "worker", "recipient": recipient, "body": "hello",
            "status": status, "revision": revision, "supersedes": supersedes,
            "superseded_by": by,
        }))
    };
    let second = row(2, "5", "accepted", json!("abc1"), json!(1), json!(null));
    assert_eq!(
        seen,
        [
            row(
                1,
                "parent",
                "superseded",
                json!(null),
                json!(null),
                json!(2)
            ),
            second.clone(),
            None,
            second,
            None,
        ]
    );
    let keys: Vec<String> = seen[1]
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|row| row.keys().cloned().collect())
        .unwrap();
    assert_eq!(
        keys,
        [
            "id",
            "sender",
            "recipient",
            "body",
            "status",
            "revision",
            "supersedes",
            "superseded_by"
        ]
    );
}

/// #2276: the inbox flag is bound into `(status='accepted' OR ?)` as
/// given, so SQLite's truth of it decides; the inbox is the recipient's,
/// in id order, at most a hundred; a list flag is refused naming
/// parameter 2, and an id beyond i64 naming parameter 1.
#[test]
fn the_inbox_binds_its_flag_as_python_does() {
    let dir = tempfile::tempdir().unwrap();
    let (repository, database) = repository(&dir);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<104)
             INSERT INTO messages(sender,recipient,body,status)
             SELECT 'worker', CASE WHEN i=2 THEN 'other' ELSE 'parent' END, 'x',
                    CASE WHEN i<=3 THEN 'consumed' ELSE 'accepted' END FROM n;",
        )
        .unwrap();
    let mut counts = Vec::new();
    let mut refusals = Vec::new();
    repository
        .atomic(false, &mut |transaction| {
            for flag in [
                json!(false),
                json!(true),
                json!("abc"),
                json!("1"),
                json!(null),
                json!(0.5),
            ] {
                let inbox = transaction.inbox("parent", &flag)?;
                let ids: Vec<i64> = inbox
                    .iter()
                    .map(|row| row.get("id").and_then(serde_json::Value::as_i64).unwrap())
                    .collect();
                assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
                counts.push((inbox.len(), ids.first().copied()));
            }
            refusals.push(transaction.inbox("parent", &json!([1])).unwrap_err());
            refusals.push(
                transaction
                    .message(&json!(9_223_372_036_854_775_808_u64))
                    .unwrap_err(),
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        counts,
        [
            (100, Some(4)),
            (100, Some(1)),
            (100, Some(4)),
            (100, Some(1)),
            (100, Some(4)),
            (100, Some(1)),
        ]
    );
    let texts: Vec<&str> = refusals.iter().map(|refusal| refusal.message()).collect();
    assert!(
        texts[0].ends_with("Error binding parameter 2: type 'list' is not supported"),
        "{texts:?}"
    );
    assert!(
        texts[1].ends_with(
            "Error binding parameter 1: Python int too large to convert to SQLite INTEGER"
        ),
        "{texts:?}"
    );
}
