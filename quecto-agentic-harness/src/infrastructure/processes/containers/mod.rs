//! Container-script process adapters (#2024 S4b, S4d): the bounded,
//! sanitised stderr every script invocation keeps for its failure report,
//! the create script's `--preflight-only` run behind the environments
//! capability's preflight port, the retained `inspect`/`cleanup` runners,
//! the synchronous liveness adapter behind the registry restore and the
//! collector, and the inventory the collector reads through a config's own
//! scripts (`inspect --list`, `cleanup`, the state roots). Mechanics only:
//! which script runs, and what its findings mean, is decided in the
//! application layer; no runtime is named here.

pub mod environment_process;
pub mod preflight;
pub mod retained_scripts;
pub mod script_inventory;
pub mod script_stderr;
