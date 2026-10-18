//! The providers a login may name (#1839): an allowlist, matched exactly.

/// A provider `auth login` can store a credential for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginProvider {
    OpenAi,
    Anthropic,
    Xai,
}

impl LoginProvider {
    /// Every login provider, in the order a refusal lists them.
    pub const ALL: [LoginProvider; 3] = [
        LoginProvider::OpenAi,
        LoginProvider::Anthropic,
        LoginProvider::Xai,
    ];

    /// The provider named exactly `name`, or `None` for any other name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.id() == name)
    }

    /// Whether a device-code login is offered for this provider. Only
    /// OpenAI's: Anthropic has no device-code endpoint, and xAI's flow is
    /// withheld because the grant is never polled for a token, so offering
    /// it would report a login that never happens (PR #1087 review).
    pub fn offers_device_code(self) -> bool {
        match self {
            LoginProvider::OpenAi => true,
            LoginProvider::Anthropic | LoginProvider::Xai => false,
        }
    }

    /// The provider's id: its credential's key in the store.
    pub fn id(self) -> &'static str {
        match self {
            LoginProvider::OpenAi => "openai",
            LoginProvider::Anthropic => "anthropic",
            LoginProvider::Xai => "xai",
        }
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
