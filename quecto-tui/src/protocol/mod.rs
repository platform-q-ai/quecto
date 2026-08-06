//! TUI protocol boundary.
//!
//! Owns the UDS client, wire DTOs, raw framing/deserialization, and
//! DTO-to-feature mapping. Feature modules and views consume typed results
//! from these mappers rather than hand-parsing wire JSON.
//!
//! This module must not import `components`, `shell`, or other feature modules
//! (`conversation`, `sessions`, `agents`, `workflow`, `inference`, `workspace`,
//! `shell`/`components`).

pub mod agent_ledger_payloads;
pub mod client;
pub mod model_payloads;
pub mod presentation_payloads;
pub mod range_accumulator;
pub mod session_payloads;
pub mod state_payloads;
pub mod subagent_payloads;
pub mod workflow_payloads;
