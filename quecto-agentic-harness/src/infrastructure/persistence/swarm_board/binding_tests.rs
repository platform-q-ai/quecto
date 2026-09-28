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

/// `SELECT typeof(?1), hex(?1), length(?1)` for each JSON text's value, as
/// Python's `sqlite3` binds `json.loads(text)` (captured from Python 3.14.7
/// with SQLite 3.53.4): a NaN is stored as NULL, `-0.0` reads back as a
/// REAL whose text is `0.0`, and a string keeps its embedded NUL.
const PYTHON_BINDINGS: &[(&str, &str, &str, Option<i64>)] = &[
    ("null", "null", "", None),
    ("true", "integer", "31", Some(1)),
    ("false", "integer", "30", Some(1)),
    (
        "-9223372036854775808",
        "integer",
        "2D39323233333732303336383534373735383038",
        Some(20),
    ),
    (
        "9223372036854775807",
        "integer",
        "39323233333732303336383534373735383037",
        Some(19),
    ),
    ("2.5", "real", "322E35", Some(3)),
    ("\"x\"", "text", "78", Some(1)),
    ("NaN", "null", "", None),
    ("Infinity", "real", "496E66", Some(3)),
    ("-Infinity", "real", "2D496E66", Some(4)),
    ("-0.0", "real", "302E30", Some(3)),
    ("\"a\\u0000b\"", "text", "610062", Some(1)),
];

#[test]
fn each_json_scalar_binds_as_the_sqlite_type_python_gives_it() {
    let connection = Connection::open_in_memory().expect("in-memory SQLite opens");
    for &(text, kind, hex, length) in PYTHON_BINDINGS {
        let bound = bind(&json(text)).expect("a scalar binds");
        let stored: (String, String, Option<i64>) = connection
            .query_row("SELECT typeof(?1), hex(?1), length(?1)", [&bound], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("the bound value reads back");
        assert_eq!(stored, (kind.to_owned(), hex.to_owned(), length), "{text}");
    }
    let cases = [
        ("null", Value::Null),
        ("true", Value::Integer(1)),
        ("false", Value::Integer(0)),
        ("-9223372036854775808", Value::Integer(i64::MIN)),
        ("9223372036854775807", Value::Integer(i64::MAX)),
        ("2.5", Value::Real(2.5)),
        ("Infinity", Value::Real(f64::INFINITY)),
        ("\"a\\u0000b\"", Value::Text("a\0b".into())),
    ];
    for (text, value) in cases {
        assert_eq!(bind(&json(text)), Ok(value), "{text}");
    }
    let negative_zero = bind(&json("-0.0"));
    assert!(
        matches!(negative_zero, Ok(Value::Real(zero)) if zero == 0.0 && zero.is_sign_negative()),
        "-0.0 binds as a REAL -0.0: {negative_zero:?}"
    );
    let nan = bind(&json("NaN"));
    assert!(
        matches!(nan, Ok(Value::Real(value)) if value.is_nan()),
        "NaN binds as a REAL NaN: {nan:?}"
    );
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

#[test]
fn a_float_reaching_text_affinity_reads_as_the_bundled_sqlite_writes_it() {
    // Deliberate divergence (M1): a REAL becomes TEXT by the linked SQLite's
    // own conversion, and this version's (3.53, as Arch's system library)
    // writes 17 significant digits where Debian's 3.46.1 and Ubuntu 24.04's
    // 3.45.1 write 15 ("0.333333333333333"). Board SQL reaches it when a
    // loosely typed float meets a TEXT column, e.g. a float `recipient`.
    let connection = table();
    let third = bind(&json("0.3333333333333333")).expect("a float binds");
    assert_eq!(third, Value::Real(1.0 / 3.0));
    let (cast, concatenated): (String, String) = connection
        .query_row("SELECT CAST(?1 AS TEXT), ?1 || ''", [&third], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("the conversions run");
    assert_eq!(cast, "0.33333333333333332");
    assert_eq!(concatenated, "0.33333333333333332");
    connection
        .execute("UPDATE tasks SET title=? WHERE id=3", [&third])
        .expect("the float is stored");
    let stored: (String, String) = connection
        .query_row(
            "SELECT typeof(title), title FROM tasks WHERE id=3",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the title reads");
    assert_eq!(stored, ("text".into(), "0.33333333333333332".into()));
}
