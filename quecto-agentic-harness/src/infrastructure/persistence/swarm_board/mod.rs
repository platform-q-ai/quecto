//! The swarm board's persistence (epic #2265).
//!
//! The board lives in a SQLite file that the Python implementation also
//! reads and writes, so everything here reproduces Python's observable
//! behaviour and bytes: the JSON codec ([`py_json`]), the verbatim schema
//! ([`schema`]), Python `sqlite3`'s parameter binding ([`binding`]) and the
//! connection and transaction discipline ([`store`]), with the request
//! ledger and event log ([`ledger`]). The application's board ports are
//! implemented over the store by [`repository`] (#2270), with the id source
//! ([`ids`]) and the encoding the board bounds arguments by ([`encoding`]).

pub mod binding;
pub mod encoding;
pub mod ids;
pub mod ledger;
pub mod meter;
pub mod py_json;
pub mod repository;
mod repository_control;
mod repository_evidence;
mod repository_members;
mod repository_reservations;
mod repository_tasks;
mod repository_usage;
pub mod schema;
pub mod store;
