//! SPIKE (board service, branch `spike/board-sqlite`, never merged): the
//! project board in a SQLite database served by `quecto board-server`,
//! instead of the `quecto/board` git branch. Disposable code that proves
//! or disproves the design with measurements.
pub mod bench;
pub mod client;
pub mod files;
pub mod http;
pub mod lease;
pub mod server;
pub mod sqlite;
pub mod store;
pub mod timing;
pub mod tools;
pub mod wire;
