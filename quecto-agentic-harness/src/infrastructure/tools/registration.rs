use std::borrow::Cow;

use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolAvailability, ToolRestrictionReason, ToolSource,
};
use crate::domain::tool_id::ToolIdentity;

/// Ownership and lifecycle metadata supplied when a tool enters the common
/// registry. Delivery adapters (bundled native, UDS, future sources) differ only
/// in this metadata and in the concrete `Tool` implementation/proxy they supply.
#[derive(Debug, Clone)]
pub struct ToolRegistration {
    pub source: ToolSource,
    pub owner: Cow<'static, str>,
    pub provider_id: Cow<'static, str>,
    pub availability: ToolAvailability,
    pub default_enabled: bool,
    pub configured_enabled: Option<bool>,
    pub configured_scope: Option<ProfileAvailabilityScope>,
    pub profile_enabled: Option<bool>,
    pub profile_scope: Option<ProfileAvailabilityScope>,
    pub session_enabled: Option<bool>,
    /// Non-widenable profile ceiling inherited by a spawned child at process launch.
    pub inherited_scope: Option<ProfileAvailabilityScope>,
    pub explicit_restriction: Option<ToolRestrictionReason>,
    /// Whether lifecycle APIs may unregister this concrete registration without
    /// removing/denying the stable tool name. UDS tools are unloadable when their
    /// connection unregisters or disconnects; bundled native tools are not.
    pub unloadable: bool,
    pub aliases: Vec<Cow<'static, str>>,
    pub stable_id_override: Option<Cow<'static, str>>,
}

impl ToolRegistration {
    pub fn official_native() -> Self {
        Self {
            source: ToolSource::BundledNative,
            owner: Cow::Borrowed("quecto:official-tools"),
            provider_id: Cow::Borrowed("quecto:official-tools"),
            availability: ToolAvailability::Enabled,
            default_enabled: true,
            configured_enabled: None,
            configured_scope: None,
            profile_enabled: None,
            profile_scope: None,
            session_enabled: None,
            inherited_scope: None,
            explicit_restriction: None,
            unloadable: false,
            aliases: Vec::new(),
            stable_id_override: None,
        }
    }

    pub fn uds() -> Self {
        Self::uds_owner("uds:runtime")
    }

    pub fn uds_owner(owner: impl Into<Cow<'static, str>>) -> Self {
        let owner = owner.into();
        Self {
            source: ToolSource::Uds,
            provider_id: owner.clone(),
            owner,
            availability: ToolAvailability::Enabled,
            default_enabled: true,
            configured_enabled: None,
            configured_scope: None,
            profile_enabled: None,
            profile_scope: None,
            session_enabled: None,
            inherited_scope: None,
            explicit_restriction: None,
            unloadable: true,
            aliases: Vec::new(),
            stable_id_override: None,
        }
    }

    pub fn runtime(owner: impl Into<Cow<'static, str>>) -> Self {
        let owner = owner.into();
        Self {
            source: ToolSource::Runtime,
            provider_id: owner.clone(),
            owner,
            availability: ToolAvailability::Enabled,
            default_enabled: true,
            configured_enabled: None,
            configured_scope: None,
            profile_enabled: None,
            profile_scope: None,
            session_enabled: None,
            inherited_scope: None,
            explicit_restriction: None,
            unloadable: true,
            aliases: Vec::new(),
            stable_id_override: None,
        }
    }

    pub fn with_availability(mut self, availability: ToolAvailability) -> Self {
        self.availability = availability;
        self
    }

    pub fn with_provider_id(mut self, provider_id: impl Into<Cow<'static, str>>) -> Self {
        self.provider_id = provider_id.into();
        self
    }

    pub fn with_entrypoint_default_enabled(mut self, enabled: bool) -> Self {
        self.default_enabled = enabled;
        self.availability = if enabled {
            ToolAvailability::Enabled
        } else {
            ToolAvailability::Disabled
        };
        self.explicit_restriction = if enabled {
            None
        } else {
            Some(ToolRestrictionReason::EntrypointDefault)
        };
        self
    }

    pub fn with_session_enabled(mut self, enabled: bool, reason: ToolRestrictionReason) -> Self {
        self.session_enabled = Some(enabled);
        self.availability = if enabled {
            ToolAvailability::Enabled
        } else {
            ToolAvailability::Disabled
        };
        self.explicit_restriction = if enabled { None } else { Some(reason) };
        self
    }

    pub fn with_spawn_restriction(mut self) -> Self {
        self.session_enabled = Some(false);
        self.availability = ToolAvailability::Disabled;
        self.explicit_restriction = Some(ToolRestrictionReason::Spawn);
        self
    }

    pub fn unloadable(mut self, unloadable: bool) -> Self {
        self.unloadable = unloadable;
        self
    }

    pub fn with_alias(mut self, alias: impl Into<Cow<'static, str>>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn with_stable_id(mut self, stable_id: impl Into<Cow<'static, str>>) -> Self {
        self.stable_id_override = Some(stable_id.into());
        self
    }

    pub fn identity_for_name(&self, name: &str) -> ToolIdentity {
        let mut identity = ToolIdentity::new(
            self.source,
            self.provider_id.as_ref(),
            name,
            self.aliases.clone(),
        );
        if let Some(stable_id) = &self.stable_id_override {
            identity.stable_id = stable_id.clone();
        }
        identity
    }
}
