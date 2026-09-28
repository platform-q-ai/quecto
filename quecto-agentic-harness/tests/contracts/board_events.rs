//! `BoardEvents` on the SQLite adapter (#2270): `Store.event` rows — the
//! actor, a REAL time, the action and the detail in the board's encoding —
//! and the control generation, the latest `paused`/`resumed` event id.
use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::BoardRepository;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

#[test]
fn events_are_logged_encoded_and_generations_follow_pauses() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository
        .atomic(true, &mut |transaction| {
            assert_eq!(transaction.control_generation()?, 0);
            transaction.event("parent", 1.0, "created", &json!({"z": 1, "a": [2.5, "é"]}))?;
            transaction.event("parent", 2.0, "paused", &json!({"started": 2.0}))?;
            transaction.event("worker", 3.0, "claimed", &json!({}))?;
            assert_eq!(transaction.control_generation()?, 2);
            transaction.event("parent", 4.0, "resumed", &json!({}))?;
            assert_eq!(transaction.control_generation()?, 4);
            Ok(())
        })
        .unwrap();
    let rows: Vec<(i64, String, String, String, String)> = rusqlite::Connection::open(&database)
        .unwrap()
        .prepare("SELECT id, actor, typeof(time), action, detail FROM events ORDER BY id")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0],
        (
            1,
            "parent".to_owned(),
            "real".to_owned(),
            "created".to_owned(),
            r#"{"a":[2.5,"\u00e9"],"z":1}"#.to_owned()
        )
    );
    assert_eq!(rows[1].4, r#"{"started":2.0}"#);
}
