//! The swarm board's schema, copied verbatim from
//! `swarm_helpers/swarm_store.py` (#2269).
//!
//! SQLite stores each `CREATE` statement's text in `sqlite_master`, and a
//! board may be created by either implementation, so [`SCHEMA`] is Python's
//! string byte for byte, including its line breaks and its leading space
//! continuations. Tables created lazily by later operations
//! (`wake_cursors`, `request_usage`, `usage_budget`) are not part of it:
//! they are created where Python creates them.

/// `swarm_store.SCHEMA`: executed on create, split on `;`, one non-blank
/// statement at a time.
pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run (id TEXT PRIMARY KEY, goal TEXT, constraints TEXT, criteria TEXT,
 coordinator TEXT, integrator TEXT, member_limit INTEGER, deadline REAL, status TEXT);
CREATE TABLE IF NOT EXISTS members (id TEXT PRIMARY KEY, reservation TEXT UNIQUE, status TEXT,
 pid INTEGER, started TEXT, socket TEXT);
CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, title TEXT, acceptance TEXT, dependencies TEXT,
 status TEXT, owner TEXT, token TEXT, evidence TEXT, blocker TEXT);
CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, task INTEGER, owner TEXT, claim TEXT, token TEXT);
CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY, sender TEXT, recipient TEXT, body TEXT, status TEXT);
CREATE TABLE IF NOT EXISTS evidence (criterion TEXT, artifact TEXT, revision TEXT, kind TEXT,
 actor TEXT, accepted INTEGER, PRIMARY KEY(criterion, actor));
CREATE TABLE IF NOT EXISTS requests (actor TEXT, request TEXT, payload TEXT, result TEXT,
 PRIMARY KEY(actor, request));
CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY, actor TEXT, time REAL, action TEXT, detail TEXT);
CREATE INDEX IF NOT EXISTS events_by_actor ON events(actor,id);
CREATE TABLE IF NOT EXISTS notification_cursors (actor TEXT PRIMARY KEY, event INTEGER);
";

/// One table's columns added after the first release, in Python's order.
pub type AddedColumns = (&'static str, &'static [(&'static str, &'static str)]);

/// `swarm_store.ADDED_COLUMNS`, in Python's dict order: the columns added
/// after the first release, migrated in place on every transaction.
/// #1729 a paused run may hold the outcome the coordinator proposed; #1837
/// a message may name a revision and supersede an earlier one; #1961 a
/// member records the harness that reserved (launched) it.
pub const ADDED_COLUMNS: &[AddedColumns] = &[
    ("run", &[("outcome", "TEXT"), ("outcome_reason", "TEXT")]),
    (
        "messages",
        &[
            ("revision", "TEXT"),
            ("supersedes", "INTEGER"),
            ("superseded_by", "INTEGER"),
        ],
    ),
    ("members", &[("launcher", "TEXT")]),
];

/// The non-blank statements of [`SCHEMA`], as Python's
/// `for statement in SCHEMA.split(';'): if statement.strip()` yields them.
pub fn schema_statements() -> impl Iterator<Item = &'static str> {
    SCHEMA
        .split(';')
        .filter(|statement| statement.chars().any(|c| !c.is_ascii_whitespace()))
}
