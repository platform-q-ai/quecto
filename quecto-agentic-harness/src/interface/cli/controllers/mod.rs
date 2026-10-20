//! CLI controllers (#2371, L13): parse a command line, invoke the use case
//! or handle it names, and hand the result to the terminal.

pub(super) mod agent;
pub(super) mod commands;
pub(super) mod config_cmd;
pub(super) mod config_flag;
pub(super) mod container;
pub(super) mod container_inventory;
pub(super) mod container_setup;
pub(super) mod help;
pub(super) mod models;
