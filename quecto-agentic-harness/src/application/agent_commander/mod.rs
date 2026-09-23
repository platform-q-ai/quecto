//! Agent Commander (SPIKE — never merged): a dry-run observer beside the
//! agent loop. The loop reports what happened; the commander judges it and
//! records what it would have done. Nothing it decides reaches an agent or
//! the owner.
pub mod ports;
