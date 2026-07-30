use std::borrow::Cow;

use crate::domain::tool::ToolDefinition;

/// How a registered tool is delivered into the common Quecto tool model.
///
/// The source is metadata for policy, UI, and protocol descriptors only. It is
/// not a second execution model: every source still registers a normal
/// [`crate::domain::tool::Tool`] in the same registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
