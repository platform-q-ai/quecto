//! Container-script process adapters (#2024 S4b): the bounded, sanitised
//! stderr every script invocation keeps for its failure report, and the
//! create script's `--preflight-only` run behind the environments
//! capability's preflight port. Mechanics only: which script runs, and
//! what its findings mean, is decided in the application layer.

pub mod preflight;
pub mod script_stderr;
pub mod standard;
