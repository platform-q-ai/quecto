//! Admission operation capability (#2024 S3, owner epic #1910): the
//! agent-operable side of the one host-wide inference-admission broker —
//! inspect and reset the running authority, install and uninstall its
//! systemd user service, and decide how a starting process joins it.
//!
//! The application owns the policy (idempotent install/uninstall, the
//! child-inherits-the-parent rule, the unit template) and expresses what it
//! needs of the outside world as capability-local ports: an authority admin
//! channel and a service manager. Infrastructure implements them; the
//! interface parses one command into one use case and presents the outcome;
//! composition constructs the handles.

pub mod dto;
pub mod ports;
pub mod use_cases;
