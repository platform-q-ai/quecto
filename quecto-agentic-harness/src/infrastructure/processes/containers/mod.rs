//! Container-script process adapters (#2024 S4b): the bounded, sanitised
//! stderr every script invocation keeps for its failure report, the create
//! script's `--preflight-only` run behind the environments capability's
//! preflight port, and the container-config lookup the doctor resolves
//! its target through. Mechanics only: which script runs, and what its
//! findings mean, is decided in the application layer.

pub mod config_lookup;
pub mod preflight;
pub mod script_stderr;
