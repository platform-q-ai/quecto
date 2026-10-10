use std::path::Path;
use rusqlite::{params, Connection};
use crate::application::project_board::ports::BoardSnapshot;

/// Rebuilds when the indexed head differs from the snapshot's.
pub fn refresh(path: &Path, snapshot: &BoardSnapshot) -> rusqlite::Result<bool> {
    std::fs::create_dir_all(path.parent().unwrap()).ok();
    let mut db = Connection::open(path)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS meta(head TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, status TEXT NOT NULL, parent TEXT, title TEXT NOT NULL);")?;
    let head: Option<String> = db.query_row("SELECT head FROM meta", [], |r| r.get(0)).ok();
    if head.as_deref() == Some(snapshot.head.as_str()) { return Ok(false); }
    let tx = db.transaction()?;
    tx.execute("DELETE FROM tasks", [])?;
    tx.execute("DELETE FROM meta", [])?;
    for t in &snapshot.tasks {
        let status = serde_json::to_value(t.status).unwrap().as_str().unwrap().to_string();
        tx.execute("INSERT INTO tasks VALUES (?1, ?2, ?3, ?4)", params![t.id, status, t.parent, t.title])?;
    }
    tx.execute("INSERT INTO meta VALUES (?1)", params![snapshot.head])?;
    tx.commit()?;
    Ok(true)
}

pub fn by_status(path: &Path, status: &str) -> rusqlite::Result<Vec<String>> {
    let db = Connection::open(path)?;
    let mut st = db.prepare("SELECT id FROM tasks WHERE status = ?1 ORDER BY id")?;
    st.query_map([status], |r| r.get(0))?.collect()
}
