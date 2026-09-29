//! The swarm coordination board's persistence (epic #2265).
//!
//! The board lives in a SQLite file that the Python implementation also
//! reads and writes, so everything here reproduces Python's observable
//! bytes. This slice holds only the JSON codec; the store and the schema
//! follow in later slices.

pub mod py_json;
