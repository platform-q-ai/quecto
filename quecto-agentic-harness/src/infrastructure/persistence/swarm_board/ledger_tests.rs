use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use super::{event, retry};
use crate::infrastructure::persistence::swarm_board::py_json::{self, PyJson};
use crate::infrastructure::persistence::swarm_board::store::{BoardStore, StoreRefusal};

fn json(text: &str) -> PyJson {
    py_json::decode(text).expect("test JSON is valid")
}

fn created(dir: &tempfile::TempDir) -> BoardStore {
    let store = BoardStore::new(dir.path().join("swarm.sqlite"));
    store
        .transaction(true, |_| Ok(()))
        .expect("the board is created");
    store
}

/// A plain second connection, as another process would hold one.
fn other(path: &Path) -> Connection {
    let connection = Connection::open(path).expect("a second connection opens");
    connection
        .busy_timeout(Duration::from_millis(500))
        .expect("busy timeout set");
    connection
}

#[test]
fn retry_replays_the_stored_result_and_refuses_a_different_payload() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let payload = json("{\"title\":\"t\",\"acceptance\":[\"pass\"]}");
    let mut actions = 0;
    let first = store
        .transaction(false, |tx| {
            retry(tx, "worker", "r1", &payload, || {
                actions += 1;
                Ok(json("{\"id\":1,\"title\":\"t\"}"))
            })
        })
        .expect("a new request runs");
    assert_eq!(first, json("{\"id\":1,\"title\":\"t\"}"));
    let stored: (String, String, String, String) = other(store.path())
        .query_row("SELECT * FROM requests", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("the request is recorded");
    assert_eq!(
        stored,
        (
            "worker".into(),
            "r1".into(),
            "{\"acceptance\":[\"pass\"],\"title\":\"t\"}".into(),
            "{\"id\":1,\"title\":\"t\"}".into()
        )
    );

    let replayed = store
        .transaction(false, |tx| {
            retry(tx, "worker", "r1", &payload, || {
                actions += 1;
                Ok(json("null"))
            })
        })
        .expect("a retry replays");
    assert_eq!((replayed, actions), (first, 1), "the action ran once");

    let reused = store.transaction(false, |tx| {
        retry(tx, "worker", "r1", &json("{\"title\":\"u\"}"), || {
            Ok(json("null"))
        })
    });
    assert_eq!(
        reused,
        Err(StoreRefusal(
            "request id reused with different payload".into()
        ))
    );
    let other_actor = store
        .transaction(false, |tx| {
            retry(tx, "coordinator", "r1", &json("{}"), || Ok(json("2")))
        })
        .expect("request ids are per actor");
    assert_eq!(other_actor, json("2"));

    for bad in [
        "",
        " \t\n",
        "\u{1c}\u{1f}\u{3000}",
        &"x".repeat(129),
        &"\u{e9}".repeat(65),
    ] {
        let refused = store.transaction(false, |tx| {
            retry(tx, "worker", bad, &json("{}"), || Ok(json("null")))
        });
        assert_eq!(
            refused,
            Err(StoreRefusal(
                "request id must be nonempty and at most 128 bytes".into()
            )),
            "{bad:?}"
        );
    }
    store
        .transaction(false, |tx| {
            retry(tx, "worker", &"\u{e9}".repeat(64), &json("{}"), || {
                Ok(json("null"))
            })
        })
        .expect("128 bytes is allowed");
}

#[test]
fn retry_refuses_when_the_ledger_holds_10000_requests() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    store
        .transaction(false, |tx| {
            let mut insert =
                tx.prepare("INSERT INTO requests VALUES('filler', ?, '{}', 'null')")?;
            for index in 0..9_999 {
                insert.execute([index.to_string()])?;
            }
            Ok(())
        })
        .expect("9999 requests are recorded");
    store
        .transaction(false, |tx| {
            retry(tx, "worker", "last", &json("{}"), || Ok(json("1")))
        })
        .expect("the 10000th request fits");
    let mut ran = false;
    let full = store.transaction(false, |tx| {
        retry(tx, "worker", "one-more", &json("{}"), || {
            ran = true;
            Ok(json("1"))
        })
    });
    assert_eq!(
        full,
        Err(StoreRefusal(
            "coordination request ledger full (10000)".into()
        ))
    );
    assert!(!ran, "a full ledger refuses before the action");
    let replayed = store
        .transaction(false, |tx| {
            retry(tx, "worker", "last", &json("{}"), || Ok(json("2")))
        })
        .expect("a recorded request still replays");
    assert_eq!(replayed, json("1"));
}

#[test]
fn event_records_the_actor_the_clock_and_the_encoded_detail() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    store
        .transaction(false, |tx| {
            event(
                tx,
                "worker",
                1_700_000_000.25,
                "claimed",
                &json("{\"task\":3,\"b\":[1.0,\"\u{e9}\"]}"),
            )
        })
        .expect("the event is recorded");
    let row: (i64, String, f64, String, String, String) = other(store.path())
        .query_row(
            "SELECT id, actor, time, action, detail, typeof(time) FROM events",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("the event reads");
    assert_eq!(
        row,
        (
            1,
            "worker".into(),
            1_700_000_000.25,
            "claimed".into(),
            "{\"b\":[1.0,\"\\u00e9\"],\"task\":3}".into(),
            "real".into()
        )
    );
}

/// Records request `r` for `worker` with payload `{}` and the raw `result`.
fn stored_request(
    store: &BoardStore,
    payload: rusqlite::types::Value,
    result: rusqlite::types::Value,
) {
    store
        .transaction(false, |tx| {
            tx.execute(
                "INSERT INTO requests VALUES('worker', 'r', ?, ?)",
                rusqlite::params![payload, result],
            )?;
            Ok(())
        })
        .expect("the request is recorded");
}

fn replay(store: &BoardStore) -> Result<PyJson, StoreRefusal> {
    store.transaction(false, |tx| {
        retry(tx, "worker", "r", &json("{}"), || Ok(json("null")))
    })
}

#[test]
fn a_stored_result_is_read_as_json_loads_reads_the_column() {
    use rusqlite::types::Value;
    let payload = || Value::Text("{}".into());
    let cases: [(Value, Result<PyJson, StoreRefusal>); 6] = [
        (Value::Blob(b"{\"id\":1}".to_vec()), Ok(json("{\"id\":1}"))),
        (Value::Blob(b"\xef\xbb\xbf[2]".to_vec()), Ok(json("[2]"))),
        // TEXT affinity stores a number as its text, which json.loads reads.
        (Value::Integer(3), Ok(json("3"))),
        (Value::Real(3.5), Ok(json("3.5"))),
        (
            Value::Null,
            Err(StoreRefusal(
                "the JSON object must be str, bytes or bytearray, not NoneType".into(),
            )),
        ),
        (
            Value::Text("{\"id\":".into()),
            Err(StoreRefusal(
                "Expecting value: line 1 column 7 (char 6)".into(),
            )),
        ),
    ];
    for (result, expected) in cases {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = created(&dir);
        stored_request(&store, payload(), result.clone());
        assert_eq!(replay(&store), expected, "{result:?}");
    }
}

#[test]
fn an_undecodable_stored_text_is_python_s_decode_error() {
    use rusqlite::types::Value;
    let undecodable = || Value::Blob(b"\xffA".to_vec());
    let as_text = "CAST(? AS TEXT)";
    for (column, payload, result) in [("payload", as_text, "?"), ("result", "?", as_text)] {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = created(&dir);
        let (payload_value, result_value) = match column {
            "payload" => (undecodable(), Value::Text("null".into())),
            _ => (Value::Text("{}".into()), undecodable()),
        };
        store
            .transaction(false, |tx| {
                tx.execute(
                    &format!("INSERT INTO requests VALUES('worker', 'r', {payload}, {result})"),
                    rusqlite::params![payload_value, result_value],
                )?;
                Ok(())
            })
            .expect("the request is recorded");
        assert_eq!(
            replay(&store),
            Err(StoreRefusal(format!(
                "coordination store unavailable or contended: Could not decode to UTF-8 column '{column}' with text '\u{fffd}A'"
            ))),
            "{column}"
        );
    }
}
