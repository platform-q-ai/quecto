use rusqlite::{Connection, params_from_iter, types::Value};

use super::{BindingError, bind, bind_all};
use crate::infrastructure::persistence::swarm_board::py_json::{self, PyJson, PyStr};

fn json(text: &str) -> PyJson {
    py_json::decode(text).expect("test JSON is valid")
}

fn table() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory SQLite opens");
    connection
        .execute_batch(
            "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, done INTEGER);
             INSERT INTO tasks VALUES (3, 'three', 1);",
        )
        .expect("fixture table is created");
    connection
}

fn ids_matching(connection: &Connection, sql: &str, parameter: &PyJson) -> Vec<i64> {
    let bound = bind(parameter).expect("a scalar binds");
    let mut statement = connection.prepare(sql).expect("query prepares");
    statement
        .query_map([bound], |row| row.get(0))
        .expect("query runs")
        .collect::<Result<_, _>>()
        .expect("rows read")
}

#[test]
fn binding_matches_python_sqlite3_affinity() {
    let connection = table();
    let by_id = "SELECT id FROM tasks WHERE id=?";
    assert_eq!(
        ids_matching(&connection, by_id, &json("\"3\"")),
        vec![3],
        "a string id matches INTEGER 3"
    );
    assert_eq!(ids_matching(&connection, by_id, &json("3")), vec![3]);
    assert_eq!(
        ids_matching(&connection, by_id, &json("3.0")),
        vec![3],
        "a REAL 3.0 compares equal to 3"
    );
    assert_eq!(
        ids_matching(&connection, by_id, &json("true")),
        Vec::<i64>::new(),
        "True is 1, not 3"
    );
    let by_done = "SELECT id FROM tasks WHERE done=?";
    assert_eq!(
        ids_matching(&connection, by_done, &json("true")),
        vec![3],
        "True binds as 1"
    );
    assert_eq!(
        ids_matching(
            &connection,
            "SELECT id FROM tasks WHERE title=?",
            &json("\"three\"")
        ),
        vec![3]
    );
}

#[test]
fn each_json_scalar_binds_as_the_sqlite_type_python_gives_it() {
    let connection = Connection::open_in_memory().expect("in-memory SQLite opens");
    let cases = [
        ("null", "null", Value::Null),
        ("true", "integer", Value::Integer(1)),
        ("false", "integer", Value::Integer(0)),
        ("-9223372036854775808", "integer", Value::Integer(i64::MIN)),
        ("9223372036854775807", "integer", Value::Integer(i64::MAX)),
        ("2.5", "real", Value::Real(2.5)),
        ("\"x\"", "text", Value::Text("x".into())),
    ];
    for (text, kind, value) in cases {
        let bound = bind(&json(text)).expect("a scalar binds");
        assert_eq!(bound, value, "{text}");
        let stored: (String, Value) = connection
            .query_row("SELECT typeof(?1), ?1", [&bound], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("the bound value reads back");
        assert_eq!(stored, (kind.to_owned(), value), "{text}");
    }
}

#[test]
fn a_list_an_object_a_huge_integer_or_a_lone_surrogate_is_refused() {
    assert_eq!(bind(&json("[1]")), Err(BindingError::Unsupported("list")));
    assert_eq!(
        bind(&json("{\"a\":1}")),
        Err(BindingError::Unsupported("dict"))
    );
    assert_eq!(
        bind(&json("9223372036854775808")),
        Err(BindingError::IntegerTooLarge)
    );
    assert_eq!(
        bind(&json("-9223372036854775809")),
        Err(BindingError::IntegerTooLarge)
    );
    let lone = PyStr::from_code_points(vec![0xDC80]).expect("a lone surrogate is a Python str");
    assert_eq!(bind(&PyJson::Str(lone)), Err(BindingError::LoneSurrogate));
}

#[test]
fn bind_all_binds_in_order_and_stops_at_the_first_refusal() {
    let connection = table();
    let bound = bind_all(&[json("\"3\""), json("true")]).expect("scalars bind");
    let id: i64 = connection
        .query_row(
            "SELECT id FROM tasks WHERE id=? AND done=?",
            params_from_iter(bound.iter()),
            |row| row.get(0),
        )
        .expect("the row matches");
    assert_eq!(id, 3);
    assert_eq!(
        bind_all(&[json("1"), json("[]"), json("{}")]),
        Err(BindingError::Unsupported("list"))
    );
}
