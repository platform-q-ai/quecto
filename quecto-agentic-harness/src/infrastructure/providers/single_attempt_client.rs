//! Explicit HTTP capability for admission-owned inference attempts.
//!
//! Reqwest's default redirect and protocol-retry policies may replay one request
//! inside a single `send`. An admission permit authorizes only one physical send.
//! Keep this proof at the infrastructure boundary, not in inward admission ports.

/// A shared HTTP client with automatic redirects and protocol retries disabled.
///
/// Construct this from the same configured builder recipe used for your ordinary
/// client so proxy, TLS, timeouts, headers, and pool settings remain intentional.
/// Only redirect and retry policy are overridden. An already-built `Client`
/// cannot be converted: reqwest cannot inspect/override its policy per request or
/// recover its builder. Callers injecting one must explicitly construct this
/// capability from their original configuration; no defaults silently replace it.
///
/// Clones share a connection pool. Redirect responses are returned to the leaf
/// and rejected as non-200 responses; retry wrappers remain the attempt owners.
#[derive(Debug, Clone)]
pub struct SingleAttemptClient(reqwest::Client);

impl SingleAttemptClient {
    pub fn build(builder: reqwest::ClientBuilder) -> Result<Self, reqwest::Error> {
        builder
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map(Self)
    }

    pub(crate) fn into_client(self) -> reqwest::Client {
        self.0
    }
}
