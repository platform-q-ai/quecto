//! The provider factory (#1849 PR 1; split out of the OAuth refresh wiring
//! by #1839): the [`ProviderFactory`] that rebuilds a concrete provider
//! around a fresh token after an OAuth refresh. Handed to the runtime
//! composition; never constructed by the interface.
//!
//! [`ProviderFactory`]: crate::infrastructure::providers::routing::refreshable::ProviderFactory

/// Build a [`ProviderFactory`] that re-creates a provider with a new API key.
///
/// The factory knows the provider name and API base URL, and creates the
/// correct provider type (Codex for OpenAI OAuth, standard otherwise).
pub fn make_provider_factory(
    provider_name: &str,
    api_base: Option<String>,
    http_client: reqwest::Client,
) -> crate::infrastructure::providers::routing::refreshable::ProviderFactory {
    make_bounded_provider_factory(provider_name, api_base, Default::default(), http_client)
}

/// [`make_provider_factory`] for a provider configured with its own stream
/// idle bounds (#2433 review), which every rebuilt provider keeps.
pub fn make_bounded_provider_factory(
    provider_name: &str,
    api_base: Option<String>,
    stream_idle: crate::infrastructure::providers::transport::stream_idle::StreamIdle,
    http_client: reqwest::Client,
) -> crate::infrastructure::providers::routing::refreshable::ProviderFactory {
    use crate::infrastructure::providers;
    use std::sync::Arc;

    let name = provider_name.to_string();
    let base = api_base;
    let binding = move || providers::ProviderBinding {
        admission: None,
        stream_idle,
    };
    Arc::new(
        move |new_token: &str| -> Arc<dyn crate::application::providers::ports::LlmProvider> {
            if name == "openai" {
                let account_id =
                    crate::infrastructure::auth::oauth::extract_openai_account_id(new_token);
                if let Some(acct) = account_id {
                    // `base` was already validated when the original provider
                    // was constructed; an invalid base cannot appear here, but
                    // degrade to the hardwired ChatGPT backend rather than
                    // panic inside the refresh path.
                    match providers::create_codex_provider_with_client_and_admission(
                        new_token.to_string(),
                        acct.clone(),
                        base.clone(),
                        http_client.clone(),
                        binding(),
                    ) {
                        Ok(p) => return p,
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                "invalid openai api_base at token refresh; using default backend"
                            );
                            return providers::create_codex_provider_with_client_and_admission(
                                new_token.to_string(),
                                acct,
                                None,
                                http_client.clone(),
                                binding(),
                            )
                            .expect("default Codex backend is always valid");
                        }
                    }
                }
            }
            match providers::create_provider_with_client_and_admission(
                &name,
                new_token.to_string(),
                base.clone(),
                http_client.clone(),
                binding(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(
                        provider = name.as_str(),
                        error = %e,
                        "failed to rebuild provider after token refresh"
                    );
                    // Return a provider that will fail — better than panicking
                    providers::create_provider_with_client_and_admission(
                        &name,
                        new_token.to_string(),
                        None,
                        http_client.clone(),
                        binding(),
                    )
                    .unwrap_or_else(|_| {
                        Arc::new(
                            crate::infrastructure::providers::openai::OpenAiProvider::new(
                                new_token.to_string(),
                                None,
                            )
                            .with_stream_idle(stream_idle),
                        )
                    })
                }
            }
        },
    )
}

#[cfg(test)]
#[path = "provider_factory_tests.rs"]
mod tests;
