//! Pure vocabulary of an extension tool invocation.
//!
//! A concrete tool implementation creates this request and a transport layer
//! forwards it to the extension client. The reply handle it travels with is
//! the application's (`application::extensions::ports::PendingToolInvocation`,
//! #1960), so this type stays free of runtime channels.

/// A single tool invocation addressed to an extension client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInvocation {
    /// Correlation id echoed by the client in its `tool_result`.
    pub tool_call_id: String,
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Arguments payload — the LLM's JSON tool-call arguments.
    pub arguments: String,
}

/// How long the agent waits for an extension tool's result (#2423): set by
/// the tool when it registers (`timeoutSeconds`), else
/// [`ExtensionToolTimeout::DEFAULT`]. Browser and computer tools need
/// longer than the default; only whole seconds within
/// [`ExtensionToolTimeout::ALLOWED_SECONDS`] are a timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionToolTimeout {
    seconds: u64,
}

impl ExtensionToolTimeout {
    /// The seconds a tool may wait: at least one, at most ten minutes.
    pub const ALLOWED_SECONDS: std::ops::RangeInclusive<u64> = 1..=600;

    /// The wait of a tool that sets none: 30 seconds.
    pub const DEFAULT: Self = Self { seconds: 30 };

    /// The timeout of `seconds`, when it is within
    /// [`Self::ALLOWED_SECONDS`].
    pub fn from_seconds(seconds: u64) -> Option<Self> {
        Self::ALLOWED_SECONDS
            .contains(&seconds)
            .then_some(Self { seconds })
    }

    pub fn seconds(self) -> u64 {
        self.seconds
    }

    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_secs(self.seconds)
    }
}

#[cfg(test)]
#[path = "extension_tool_tests.rs"]
mod tests;
