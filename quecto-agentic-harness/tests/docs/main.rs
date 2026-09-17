//! Consolidated repository/documentation invariant target (no process spawning,
//! no `test-support` requirement): repo docs, workflow docs and templates,
//! container-runtime docs, repository file reader. Each former target is a
//! module here; `--test docs repo_docs::` selects one of them.

#[path = "../common/mod.rs"]
mod common;

mod container_runtime_docs;
mod container_runtime_safety;
mod repo_docs;
mod repository_file_reader;
mod workflow_config_refactor_template;
mod workflow_config_template;
mod workflow_docs;
