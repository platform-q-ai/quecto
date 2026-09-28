//! The board's method dispatch (#2270): red-phase skeleton.
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::use_cases::{
    BootstrapRun, CreateRun, ReadRunSnapshot, ReadRunStatus,
};
use crate::domain::swarm::BoardError;

/// The `tracing` target of every board call record.
pub const TELEMETRY_TARGET: &str = "quecto::swarm_board";

/// One handle per board use case, composed once per board file.
#[derive(Clone)]
pub struct SwarmBoardHandles {
    pub create_run: Arc<CreateRun>,
    pub bootstrap_run: Arc<BootstrapRun>,
    pub read_run_status: Arc<ReadRunStatus>,
    pub read_run_snapshot: Arc<ReadRunSnapshot>,
}

impl std::fmt::Debug for SwarmBoardHandles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SwarmBoardHandles")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    CreateRun,
}

#[derive(Clone, Copy)]
struct Parameter {
    name: &'static str,
    default: Option<fn() -> Value>,
}

const fn required(name: &'static str) -> Parameter {
    Parameter {
        name,
        default: None,
    }
}

fn bind(method: Method, parameters: &[Parameter], args: Value) -> Result<Vec<Value>, BoardError> {
    let named: Vec<_> = parameters
        .iter()
        .map(|parameter| (parameter.name, parameter.default))
        .collect();
    let _ = (method, named, args);
    Err(BoardError::new("not implemented yet (#2270)"))
}

/// # Errors
/// Not implemented yet.
pub fn call(
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    args: Value,
) -> Result<Value, BoardError> {
    let _ = (handles, member, method, TELEMETRY_TARGET);
    bind(Method::CreateRun, &[required("goal")], args)?;
    Err(BoardError::new("not implemented yet (#2270)"))
}

#[cfg(test)]
#[path = "swarm_board_dispatch_tests.rs"]
mod tests;
