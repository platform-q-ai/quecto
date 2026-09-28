//! The swarm board's persistence (epic #2265).
//!
//! The board lives in a SQLite file that the Python implementation also
//! reads and writes, so everything here reproduces Python's observable
//! behaviour and bytes: the JSON codec ([`py_json`]), the verbatim schema
//! ([`schema`]), Python `sqlite3`'s parameter binding ([`binding`]) and the
//! connection and transaction discipline ([`store`]), with the request
//! ledger and event log ([`ledger`]). The repository methods follow in
//! later slices.

pub mod binding;
pub mod ledger;
pub mod py_json;
pub mod schema;
pub mod store;
