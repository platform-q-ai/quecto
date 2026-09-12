//! Process adapters (#1935): the one owner of directly spawned children,
//! the out-of-band parent-control delivery, and the protocol edge to a
//! direct child. Effects only; no teardown policy lives here.

pub mod direct_child_routing;
pub mod owned_child_supervisor;
pub mod parent_control;
pub mod parent_death_signal;
