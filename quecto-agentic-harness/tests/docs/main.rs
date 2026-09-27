//! Consolidated repository/documentation invariant target: repo docs,
//! workflow docs and templates, container-runtime docs, repository file
//! reader. Each former target is a module here; `--test docs repo_docs::`
//! selects one of them. It requires `test-support`: the container-runtime
//! docs run a fake runtime CLI written by the race-free
//! `test_support::executable` helper (#2232).

#[path = "../common/mod.rs"]
mod common;

mod container_runtime_docs;
mod repo_docs;
mod repository_file_reader;
mod workflow_config_refactor_template;
mod workflow_config_template;
mod workflow_docs;
