//! A project's board of tasks (epic #2481): the whole jobs a project plans,
//! marks ready and claims, kept as one JSON file per task on the repository's
//! `quecto/board` branch. Not the swarm's run board (`domain::swarm`).
pub mod entities;
pub mod value_objects;
