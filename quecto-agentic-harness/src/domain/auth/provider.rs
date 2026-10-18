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
