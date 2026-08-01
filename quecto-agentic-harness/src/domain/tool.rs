use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use super::error::DomainError;
use super::tool_descriptor::{
    ProfileAvailabilityScope, ToolAvailability, ToolCatalogueEntry, ToolDescriptor,
};

/// Metadata describing a tool for the LLM.
///
/// Fields use `Cow<'static, str>` so that static tool schemas (the common
/// case — 11 of 12 tools) are zero-cost clones (pointer copy), while
/// dynamic schemas (`ls` with runtime limits) use `Cow::Owned`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    /// JSON Schema string describing the parameters.
    pub parameters_schema: Cow<'static, str>,
}

/// A base64-encoded image block returned by a tool (e.g. `read` on an image file).
#[derive(Debug, Clone)]
pub struct ImageBlock {
    /// MIME type: one of `"image/png"`, `"image/jpeg"`, `"image/gif"`, `"image/webp"`.
    /// Always a static literal — avoids a heap allocation per image block.
    pub mime_type: &'static str,
    /// Base64-encoded image bytes (standard encoding, no line breaks).
    pub data: String,
}

/// The result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    /// Optional image blocks (e.g. when `read` is called on an image file).
    /// Empty for all non-image tools — zero-cost default.
    pub image_blocks: Vec<ImageBlock>,
}

/// Port: a tool the agent can invoke.
///
/// Uses `Pin<Box<dyn Future>>` return type for dyn-compatibility,
/// allowing tools to be stored in a registry as `Arc<dyn Tool>`.
///
/// # Error handling contract
///
/// `execute` distinguishes **LLM-addressable** errors from **infrastructure**
/// errors via its return type:
///
/// - `Ok(ToolResult { is_error: true, content })` — the LLM supplied bad
///   input (malformed JSON arguments, missing or invalid field, forbidden
///   path, tool-specific validation failure). The agent loop surfaces
///   `content` back to the LLM so it can read the explanation and retry.
///   **This is the preferred form for anything the LLM can fix.**
///
/// - `Err(DomainError)` — an infrastructure-level failure the LLM cannot
///   reasonably correct (OS I/O error, sandbox violation, timeout, upstream
///   service outage). The agent loop still surfaces these as tool errors,
///   but adapter authors should reserve them for genuinely external failures.
///
/// Both forms terminate the tool call with `is_error: true` from the agent
/// loop's perspective, so neither is "wrong"; the distinction is a style
/// contract for readability and future telemetry, not a functional one.
/// Adapter unit tests should assert the chosen shape to pin down behaviour.
pub trait Tool: Send + Sync {
    /// Return the tool's definition for the LLM.
    fn definition(&self) -> ToolDefinition;

    /// Notify stateful tools that the active session key changed.
    fn set_session_key(&self, _session_key: String) {}

    /// Execute the tool with JSON-encoded arguments.
    ///
    /// See the trait-level docs for the error-handling contract:
    /// prefer `Ok(ToolResult { is_error: true })` for LLM-addressable
    /// errors; use `Err` only for infrastructure failures.
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}

/// A guard that can inspect and block tool calls before execution.
///
/// Guards run in registration order before every `execute()` call on the
/// tool registry.  The first `Err` short-circuits — remaining guards and
/// the tool itself are skipped.  The error string is returned to the LLM
/// as a `ToolResult { is_error: true }` so it can act on the feedback.
pub trait ToolGuard: Send + Sync {
    /// Inspect a tool call before execution.
    /// Return `Ok(())` to allow, `Err(reason)` to block.
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String>;
}

/// Port: a read-only catalog of tools available to the model.
///
/// Use this role when callers only need schema/name visibility and should not
/// execute or mutate the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProfileContext {
    Parent,
    Child,
}

pub trait ToolCatalog: Send + Sync {
    /// Return enabled tool definitions visible to the model.
    fn definitions(&self) -> &[ToolDefinition];

    /// Return enabled tool definitions visible to the requested profile context.
    fn definitions_for(&self, _context: ToolProfileContext) -> &[ToolDefinition] {
        self.definitions()
    }

    /// Return descriptors for all registered tools, including disabled tools
    /// hidden from model-visible definitions.
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.definitions()
            .iter()
            .cloned()
            .map(|definition| {
                ToolDescriptor::enabled(
                    definition,
                    super::tool_descriptor::ToolSource::Runtime,
                    "runtime",
                )
            })
            .collect()
    }

    /// Return rich additive catalogue/effective-policy state when the backing
    /// catalogue has real metadata. The default fallback is intentionally empty:
    /// callers that need TUI/API-ready state must use a metadata-aware registry.
    fn catalogue_entries(&self) -> Vec<ToolCatalogueEntry> {
        Vec::new()
    }

    /// Return the number of registered tools without cloning definitions.
    ///
    /// Override if `definitions()` is not O(1) (e.g. lazy-building implementations).
    fn tool_count(&self) -> usize {
        self.definitions().len()
    }
}

/// Port: execution-only access to registered tools.
///
/// Uses `Pin<Box<dyn Future>>` for dyn-compatibility.
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with JSON arguments.
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}

/// Requested runtime policy state for a registered tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyMutation {
    pub name: String,
    pub availability: ToolAvailability,
    pub scope: ProfileAvailabilityScope,
    pub reason: String,
}

impl ToolPolicyMutation {
    pub fn enable(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            availability: ToolAvailability::Enabled,
            scope: ProfileAvailabilityScope::Both,
            reason: reason.into(),
        }
    }

    pub fn disable(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            availability: ToolAvailability::Disabled,
            scope: ProfileAvailabilityScope::None,
            reason: reason.into(),
        }
    }

    pub fn set_scope(
        name: impl Into<String>,
        scope: ProfileAvailabilityScope,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            availability: ToolAvailability::from(scope),
            scope,
            reason: reason.into(),
        }
    }
}

impl From<ProfileAvailabilityScope> for ToolAvailability {
    fn from(scope: ProfileAvailabilityScope) -> Self {
        if matches!(scope, ProfileAvailabilityScope::None) {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyApplyMode {
    ImmediateIfIdle,
    AtNextTurnBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyMutationStatus {
    Applied,
    AlreadyInState,
    UnknownTool,
    BlockedByRestriction,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyMutationResult {
    pub name: String,
    pub requested_availability: ToolAvailability,
    pub requested_scope: ProfileAvailabilityScope,
    pub status: ToolPolicyMutationStatus,
    pub before: Option<ToolCatalogueEntry>,
    pub after: Option<ToolCatalogueEntry>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyReconciliation {
    pub mode: ToolPolicyApplyMode,
    pub results: Vec<ToolPolicyMutationResult>,
}

/// Port: live runtime policy mutation for registered tools.
pub trait ToolPolicyMutator: Send + Sync {
    fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        ToolPolicyReconciliation {
            mode,
            results: mutations
                .iter()
                .map(|mutation| ToolPolicyMutationResult {
                    name: mutation.name.clone(),
                    requested_availability: mutation.availability,
                    requested_scope: mutation.scope,
                    status: ToolPolicyMutationStatus::UnknownTool,
                    before: None,
                    after: None,
                    reason: mutation.reason.clone(),
                })
                .collect(),
        }
    }
}

/// Port: runtime-loadable UDS tool lifecycle management.
///
/// This port is only for tools delivered by runtime lifecycle adapters such as
/// UDS. Bundled native provider tools are compiled into Quecto and remain in the
/// catalogue while policy enables/disables them; they must not use this unload
/// path.
pub use RuntimeToolLifecycleRegistry as ExtensionToolRegistry;

pub trait RuntimeToolLifecycleRegistry: Send + Sync {
    /// Return names of currently unloadable runtime tools.
    fn runtime_tool_names(&self) -> Vec<String> {
        vec![]
    }

    /// Register a runtime-loadable tool.
    fn register_runtime_tool(&mut self, _tool: std::sync::Arc<dyn Tool>) -> bool {
        false
    }

    /// Unregister a runtime-loadable tool by name.
    ///
    /// No-op if the name is not owned by a runtime provider. Bundled native tools
    /// are disabled/enabled by policy rather than unloaded. Default: no-op.
    fn unregister_runtime_tool(&mut self, _name: &str) {}

    /// Unregister runtime-loadable tools owned by one dynamic provider/client.
    fn unregister_runtime_tools_for_owner(&mut self, _owner: &str) -> Vec<String> {
        vec![]
    }

    /// Register a UDS-delivered runtime-loadable tool.
    fn register_uds_tool(&mut self, tool: std::sync::Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn extension_names(&self) -> Vec<String> {
        self.runtime_tool_names()
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn register_extension(&mut self, tool: std::sync::Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn unregister_extension(&mut self, name: &str) {
        self.unregister_runtime_tool(name)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn unregister_extensions_for_owner(&mut self, owner: &str) -> Vec<String> {
        self.unregister_runtime_tools_for_owner(owner)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    fn register_uds_extension(&mut self, tool: std::sync::Arc<dyn Tool>) -> bool {
        self.register_uds_tool(tool)
    }

    /// Return whether a UDS-delivered runtime-loadable tool with this name and
    /// owner would be accepted by the registry without mutating it.
    fn can_register_uds_tool_for_owner(&self, _name: &str, _owner: &str) -> bool {
        true
    }

    /// Register a UDS-delivered runtime-loadable tool owned by a specific UDS
    /// client/connection.
    ///
    /// Default delegates to the ownerless UDS path for registries that do not
    /// expose per-connection ownership metadata.
    fn register_uds_tool_for_owner(
        &mut self,
        tool: std::sync::Arc<dyn Tool>,
        _owner: std::borrow::Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool(tool)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    fn can_register_uds_extension_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    fn register_uds_extension_for_owner(
        &mut self,
        tool: std::sync::Arc<dyn Tool>,
        owner: std::borrow::Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    /// Runtime-enable a registered tool. Default: unsupported.
    fn enable_tool(&mut self, _name: &str) -> bool {
        false
    }

    /// Runtime-disable a registered tool. Default: unsupported.
    fn disable_tool(&mut self, _name: &str) -> bool {
        false
    }
}

/// Port: session-key propagation for stateful tools.
pub trait SessionAwareTools: Send + Sync {
    /// Notify stateful tools that the active session key changed.
    fn set_session_key(&self, _session_key: &str) {}
}

/// Port: full tool registry capability used at composition boundaries.
///
/// Most application call sites should prefer one of the role-specific ports
/// above when they need only catalog, execution, extension lifecycle, or
/// session propagation behaviour. The full registry remains as the ergonomic
/// bundle owned by `AgentLoopImpl`.
pub trait ToolRegistry:
    ToolCatalog + ToolExecutor + RuntimeToolLifecycleRegistry + SessionAwareTools + ToolPolicyMutator
{
}

#[cfg(test)]
#[path = "tool_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
