//! Capability-local ports of the swarm capability: the lifecycle ports
//! (#1940, #1960) and the coordination board's (#2270), one module per
//! concern (#2367); importers name each port at its owning file.
//!
//! The board's ports are segregated by role (ADR-0019): a use case bounds
//! only the roles it uses, and later slices add roles instead of growing one
//! transaction trait. Every role method runs inside the one transaction its
//! `BoardRepository::atomic` opened; a store failure is a `BoardError`
//! carrying the board's text (`coordination store unavailable or contended:
//! …`).
//!
//! Wire names, positional arguments and persistence schemas are adapter
//! details; every signature here names only domain types and the
//! capability's own DTOs.

use std::future::Future;
use std::pin::Pin;

pub mod board_events;
pub mod board_evidence;
pub mod board_files;
pub mod board_members;
pub mod board_messages;
pub mod board_projection;
pub mod board_repository;
pub mod board_requests;
pub mod board_runs;
pub mod board_tasks;
pub mod board_telemetry;
pub mod board_usage;
pub mod board_wakes;
pub mod lifecycle;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
