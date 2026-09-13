//! Pure launch vocabulary for subagent launch transactions.
//!
//! Per ADR-0021 the domain owns launch intent/result identities only. The
//! launch port (`SubagentLaunchPorts`) and its boxed-future alias
//! (`LaunchFuture`) are the application's (`application::subagent_launch`,
//! #1940, #1960); every side effect (process construction, sockets, script
//! execution, JSON contract parsing) lives behind its implementations in
//! infrastructure.

use std::path::PathBuf;

/// Typed parent-side endpoint for reaching a launched child (#1369 slice 3).
///
/// A launch result carries EXACTLY ONE endpoint mode: a direct UDS socket the
/// parent connects to, or a validated proxy argv the parent runs per
/// connection as a stdio<->child bridge. The prepared endpoint is carried
/// transactionally through readiness, prompt routing, commands, passive
/// completion reporting, and monitor construction — never reconstructed from a requested path or a
/// mutable registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentEndpoint {
    /// Direct UDS socket path the parent connects to.
    Direct { socket_path: PathBuf },
    /// Validated proxy argv; each parent connection runs this command and
    /// speaks the child protocol over its stdio.
    Proxy { argv: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchIdentity {
    pub session_name: String,
    pub registry_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRuntime {
    pub socket_path: PathBuf,
    pub pid: u32,
    pub environment_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredLaunch {
    pub registry_key: String,
    pub socket_path: PathBuf,
}
