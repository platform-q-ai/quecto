//! Request and result types of the admission operation capability (#2024 S3).
//! These are the application's own vocabulary; the interface maps CLI/UDS
//! shapes to and from them and never leaks a wire or systemd type inward.

pub mod authority;
pub mod broker;
pub mod negotiation;
pub mod service;
