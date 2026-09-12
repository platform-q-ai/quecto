//! Delegated-subagent teardown capability (#1934, epic #1929).
//!
//! Owns the typed contracts for self shutdown and selected-descendant
//! termination: boundary DTOs, the capability-local effect ports, and the
//! use cases that sequence them. Nothing here knows about sockets, JSON,
//! processes or persistence formats; those live in adapters that implement
//! [`ports`] and in the interface that maps wire DTOs onto [`dto`].

pub mod dto;
pub mod ports;
pub mod use_cases;
