//! Container-script process adapters (#2024 S4b, S4d): the bounded,
//! sanitised stderr every script invocation keeps for its failure report,
//! the create script's `--preflight-only` run behind the environments
//! capability's preflight port, the retained `inspect`/`cleanup` runners,
//! the synchronous liveness adapter behind the registry restore and the
//! collector, and the runtime's own inventory (`podman`/`docker ps`, state
//! roots). Mechanics only: which script runs, and what its findings mean,
//! is decided in the application layer.

pub mod environment_process;
pub mod preflight;
pub mod retained_scripts;
pub mod runtime_inventory;
pub mod script_stderr;
