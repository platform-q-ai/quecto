//! Pure tool vocabulary: definitions, results and runtime policy values.
//!
//! The tool ports (`Tool`, `ToolGuard`, `ToolCatalog`, `ToolExecutor`,
//! `ToolPolicyMutator`, `RuntimeToolLifecycleRegistry`, `SessionAwareTools`,
//! `ToolRegistry`, `ToolExecutionAdmission`) are the application's
//! (`application::tools::ports`, #1960).
use std::borrow::Cow;

use super::tool_descriptor::{ProfileAvailabilityScope, ToolAvailability, ToolCatalogueEntry};

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
    /// Internal delivery metadata passed to `Tool::result_delivered` after
    /// `content` has been appended. This is never surfaced to the model.
    pub delivery_metadata: Option<String>,
}

/// Which profile a tool catalogue is being read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProfileContext {
    Parent,
    Child,
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
pub enum ToolPolicyOperation {
    Patch,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyRequest {
    pub operation: ToolPolicyOperation,
    pub mutations: Vec<ToolPolicyMutation>,
    pub unlisted_scope: Option<ProfileAvailabilityScope>,
    pub correlation_id: Option<String>,
    pub persist: bool,
}

impl ToolPolicyRequest {
    pub fn patch(mutations: Vec<ToolPolicyMutation>) -> Self {
        Self {
            operation: ToolPolicyOperation::Patch,
            mutations,
            unlisted_scope: None,
            correlation_id: None,
            persist: false,
        }
    }

    pub fn replace(
        mutations: Vec<ToolPolicyMutation>,
        unlisted_scope: ProfileAvailabilityScope,
    ) -> Self {
        Self {
            operation: ToolPolicyOperation::Replace,
            mutations,
            unlisted_scope: Some(unlisted_scope),
            correlation_id: None,
            persist: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyMutationStatus {
    Applied,
    AlreadyInState,
    UnknownTool,
    BlockedByRestriction,
    PersistenceFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyMutationResult {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_identifier: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
