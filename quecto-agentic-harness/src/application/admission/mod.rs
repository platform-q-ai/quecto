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
//!
//! It also holds the runtime authority and attempt lifecycle (#1910): the
//! admission service's registry/client/dispatcher/recovery ports and the
//! attempt, journal/secret and observation ports in [`ports`], the
//! admission service and credential-gated authority in [`services`], and the
//! use cases that make the authority durable in [`use_cases`].

pub mod dto;
pub mod ports;
pub mod services;
pub mod use_cases;
