//! Receiving an authorization code on the local browser callback (#1839).

use crate::domain::auth::provider::LoginProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationCallbackError {
    /// No code arrived (timeout, a denied authorization, a busy port): the
    /// user may still paste one. In the listener's words.
    Failed(String),
    /// The provider's redirect URI cannot be listened on.
    InvalidRedirect(String),
    /// The listener could not run at all here (e.g. no thread could be
    /// started); in the mechanism's words.
    Unavailable(String),
    /// The caller is itself inside an async runtime, where a blocking
    /// listener cannot run.
    InsideAsyncRuntime,
}

pub trait AuthorizationCallback: Send + Sync {
    /// Wait for `provider`'s browser callback carrying `state` and return
    /// its authorization code. A callback with any other state is refused
    /// and waiting goes on; waiting is bounded. Blocking: never call it
    /// from inside an async runtime (it answers `InsideAsyncRuntime` there).
    fn receive(
        &self,
        provider: LoginProvider,
        state: &str,
    ) -> Result<String, AuthorizationCallbackError>;
}
