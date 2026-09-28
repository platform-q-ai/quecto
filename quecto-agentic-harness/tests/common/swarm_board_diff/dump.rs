//! The logical comparator: a board file as the rows it holds, each column
//! as its storage class (`typeof`) and value, so INTEGER-vs-REAL drift is a
//! difference. Page layout, the change counter and free pages are not
//! compared: byte-identical files are impossible even Python-vs-Python.
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};

/// One column: its `typeof` and its value.
pub type Cell = (String, SqlValue);

/// Each table's rows, in a stable order, by table name.
pub type Dump = Vec<(String, Vec<Vec<Cell>>)>;

/// `sqlite_master` (type, name, tbl_name, sql, in creation order), then
/// every table ordered by rowid — `files` by `path`, since Python reserves
/// in `set` order, which is hash-seeded per process — then the journal
/// mode and user version.
pub fn logical_dump(path: &Path) -> Dump {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| panic!("open {} to dump it: {error}", path.display()));
    let mut dump = vec![(
        "sqlite_master".to_owned(),
        rows(
            &connection,
            "SELECT typeof(type), type, typeof(name), name, typeof(tbl_name), tbl_name, typeof(sql), sql FROM sqlite_master ORDER BY rowid",
        ),
    )];
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in tables {
        let columns: Vec<String> = connection
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let selected: Vec<String> = columns
            .iter()
            .map(|column| format!("typeof(\"{column}\"), \"{column}\""))
            .collect();
        let order = if table == "files" {
            "\"path\""
        } else {
            "rowid"
        };
        let sql = format!(
            "SELECT {} FROM \"{table}\" ORDER BY {order}",
            selected.join(", ")
        );
        dump.push((table.clone(), rows(&connection, &sql)));
    }
    for pragma in ["journal_mode", "user_version"] {
        dump.push((
            format!("PRAGMA {pragma}"),
            rows(
                &connection,
                &format!("SELECT typeof({pragma}), {pragma} FROM pragma_{pragma}"),
            ),
        ));
    }
    dump
}

/// Rows of `(typeof, value)` column pairs.
fn rows(connection: &Connection, sql: &str) -> Vec<Vec<Cell>> {
    let mut statement = connection
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let width = statement.column_count() / 2;
    statement
        .query_map([], |row| {
            (0..width)
                .map(|index| Ok((row.get(2 * index)?, row.get(2 * index + 1)?)))
                .collect::<rusqlite::Result<Vec<Cell>>>()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The first difference between two dumps, readably, or `None`.
pub fn first_difference(python: &Dump, rust: &Dump) -> Option<String> {
    let names = |dump: &Dump| {
        dump.iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    };
    if names(python) != names(rust) {
        return Some(format!(
            "tables differ: python {:?}, rust {:?}",
            names(python),
            names(rust)
        ));
    }
    for ((table, python_rows), (_, rust_rows)) in python.iter().zip(rust) {
        if python_rows.len() != rust_rows.len() {
            return Some(format!(
                "{table}: python holds {} rows, rust {}",
                python_rows.len(),
                rust_rows.len()
            ));
        }
        for (index, (python_row, rust_row)) in python_rows.iter().zip(rust_rows).enumerate() {
            if python_row != rust_row {
                return Some(format!(
                    "{table} row {index}:\n  python {python_row:?}\n  rust   {rust_row:?}"
                ));
            }
        }
    }
    None
}
