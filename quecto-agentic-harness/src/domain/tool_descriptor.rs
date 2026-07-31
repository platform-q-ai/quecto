use std::borrow::Cow;

use serde::Serialize;

use crate::domain::tool::ToolDefinition;

/// How a registered tool is delivered into the common Quecto tool model.
///
/// The source is metadata for policy, UI, and protocol descriptors only. It is
/// not a second execution model: every source still registers a normal
/// [`crate::domain::tool::Tool`] in the same registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolSource {
    /// Official Quecto tool compiled into the binary.
    BundledNative,
    /// Tool dynamically registered by a UDS extension client.
    Uds,
    /// Tool from another runtime source not yet classified by this slice.
    Runtime,
}

impl ToolSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BundledNative => "bundled-native",
            Self::Uds => "uds",
            Self::Runtime => "runtime",
        }
    }
}

/// Runtime policy for a registered tool.
///
/// Disabled tools remain registered so they can be re-enabled without restarting
/// Quecto, but they are hidden from model-visible definitions and reject new
/// executions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolAvailability {
    Enabled,
    Disabled,
}

impl ToolAvailability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Lifecycle adapter that owns a registered tool implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolLifecycleKind {
    /// Compiled-in/native registration that policy can disable but lifecycle
    /// unload operations cannot remove.
    Bundled,
    /// Runtime-loadable registration removed by unregister/disconnect lifecycle.
    RuntimeLoadable,
}

impl ToolLifecycleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::RuntimeLoadable => "runtime-loadable",
        }
    }
}

/// Reason a tool is restricted below broader defaults/profile state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolRestrictionReason {
    EntrypointDefault,
    Session,
    Spawn,
    ExplicitDisable,
}

impl ToolRestrictionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntrypointDefault => "entrypoint-default",
            Self::Session => "session",
            Self::Spawn => "spawn",
            Self::ExplicitDisable => "explicit-disable",
        }
    }
}

/// Coarse health/status for catalogue and TUI/API readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolHealth {
    Ok,
    Disabled,
    Unavailable,
    Unknown,
}

impl ToolHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Disabled => "disabled",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
        }
    }
}

/// Descriptor surfaced to policies and UIs that need to understand tool
/// availability without depending on concrete tool implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescriptor {
    pub definition: ToolDefinition,
    pub source: ToolSource,
    pub owner: Cow<'static, str>,
    pub availability: ToolAvailability,
}

impl ToolDescriptor {
    pub fn new(
        definition: ToolDefinition,
        source: ToolSource,
        owner: impl Into<Cow<'static, str>>,
        availability: ToolAvailability,
    ) -> Self {
        Self {
            definition,
            source,
            owner: owner.into(),
            availability,
        }
    }

    pub fn enabled(
        definition: ToolDefinition,
        source: ToolSource,
        owner: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(definition, source, owner, ToolAvailability::Enabled)
    }

    pub fn name(&self) -> &str {
        self.definition.name.as_ref()
    }
}

/// Rich additive catalogue state for TUI/API callers.
///
/// This intentionally does not replace [`ToolDescriptor`] or implement the full
/// persisted profile UX. Configured/profile fields are placeholders for future
/// policy persistence, while effective fields describe the current runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogueEntry {
    pub stable_id: Cow<'static, str>,
    pub name: Cow<'static, str>,
    pub label: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub input_schema: Cow<'static, str>,
    pub source: ToolSource,
    pub owner: Cow<'static, str>,
    pub provider_id: Cow<'static, str>,
    pub version: Option<Cow<'static, str>>,
    pub lifecycle: ToolLifecycleKind,
    pub configurable: bool,
    pub default_enabled: bool,
    pub configured_enabled: Option<bool>,
    pub profile_enabled: Option<bool>,
    pub session_enabled: Option<bool>,
    pub explicit_restriction: Option<ToolRestrictionReason>,
    pub runtime_availability: ToolAvailability,
    pub effective_enabled: bool,
    pub health: ToolHealth,
}

#[cfg(test)]
#[path = "tool_descriptor_tests.rs"]
mod tests;
