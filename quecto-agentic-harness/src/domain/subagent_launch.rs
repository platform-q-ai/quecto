//! Pure launch vocabulary and port for subagent launch transactions.
//!
//! Per ADR-0021 the domain owns launch intent/result identities and the
//! launch port only. Every side effect (process construction, sockets,
//! script execution, JSON contract parsing) lives behind
//! [`SubagentLaunchPorts`] implementations in infrastructure.

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use super::error::DomainError;
use super::subagent::SubagentConfig;
use super::tool::ToolResult;

pub type LaunchFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SubagentLaunchPorts {
    type Prepared;

    fn allocate_identity(&mut self, config: &SubagentConfig)
    -> Result<LaunchIdentity, DomainError>;
    fn build_cli_args<'a>(
        &'a mut self,
        identity: &'a LaunchIdentity,
        config: &'a SubagentConfig,
    ) -> Result<Vec<OsString>, DomainError>;
    fn resolve_binary(&mut self) -> Result<PathBuf, DomainError>;
    fn prepare_child<'a>(
        &'a mut self,
        config: &'a SubagentConfig,
        binary: &'a Path,
        cli_args: &'a [OsString],
    ) -> LaunchFuture<'a, Result<Self::Prepared, DomainError>>;
    fn ready<'a>(
        &'a mut self,
        prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, Result<PreparedRuntime, DomainError>>;
    fn rollback_prepared<'a>(
        &'a mut self,
        prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, ()>;
    /// Atomically uncommit a registered launch: take/remove the registry record,
    /// cancel its monitor/ownership, terminate the runtime if appropriate, and
    /// run any claimed cleanup exactly once. This prevents prompt-failure races
    /// with monitor-owned cleanup.
    fn uncommit_registered<'a>(&'a mut self, registry_key: &'a str) -> LaunchFuture<'a, ()>;
    fn register_and_monitor<'a>(
        &'a mut self,
        identity: &'a LaunchIdentity,
        runtime: PreparedRuntime,
        prepared: &'a mut Self::Prepared,
        config: &'a SubagentConfig,
    ) -> LaunchFuture<'a, Result<RegisteredLaunch, DomainError>>;
    fn send_initial_prompt<'a>(
        &'a mut self,
        socket_path: &'a Path,
        task: &'a str,
    ) -> LaunchFuture<'a, Result<(), DomainError>>;
    fn success(&self, identity: &LaunchIdentity, environment_ref: Option<&str>) -> ToolResult;
}

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
