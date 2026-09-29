//! `BoardMessages` on the SQLite adapter (#2275): a recipient's unread
//! messages are counted by recipient and status as Python counts them, and
//! a message is inserted `accepted`, answering its id.
use quecto::application::swarm::dto::BoardLocation;
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
