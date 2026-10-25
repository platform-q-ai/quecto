//! A project's board of tasks (#2476, epic #2481): the use cases the board
//! operations (S2) invoke, over a board store and a clock. Every change is
//! a compare-and-swap on the board's head: a write that lost the race is
//! read again and the change re-applied to the fresh task, a bounded number
//! of times.
pub mod dto;
pub mod fold_back;
pub mod operations;
pub mod ports;
pub mod run_facts;
pub mod start;
pub mod swarm_brief;
pub mod synced_clock;
pub mod task_summary;
pub mod telemetry;
pub mod use_cases;
