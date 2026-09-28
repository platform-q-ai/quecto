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
pub const SCHEMA: &str = "";

/// One table's columns added after the first release, in Python's order.
pub type AddedColumns = (&'static str, &'static [(&'static str, &'static str)]);

/// `swarm_store.ADDED_COLUMNS`, in Python's dict order: the columns added
/// after the first release, migrated in place on every transaction.
/// #1729 a paused run may hold the outcome the coordinator proposed; #1837
/// a message may name a revision and supersede an earlier one; #1961 a
/// member records the harness that reserved (launched) it.
pub const ADDED_COLUMNS: &[AddedColumns] = &[];

/// The non-blank statements of [`SCHEMA`], as Python's
/// `for statement in SCHEMA.split(';'): if statement.strip()` yields them.
pub fn schema_statements() -> impl Iterator<Item = &'static str> {
    SCHEMA
        .split(';')
        .filter(|statement| statement.chars().any(|c| !c.is_ascii_whitespace()))
}
