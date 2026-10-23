//! SPIKE (board service, branch `spike/board-sqlite`, never merged): the
//! project board in a SQLite database served by `quecto board-server`,
//! instead of the `quecto/board` git branch. Disposable code that proves
//! or disproves the design with measurements.
pub mod files;
pub mod sqlite;
pub mod store;
