//! The device-code flow over the providers' OAuth endpoints (#1839): the
//! application's `DeviceCodeFlow`, run on the harness's own runtime
//! builder (#1718: a thread-exhausted process gets an error, not an abort),
//! so the CLI's per-command call needs no ambient runtime.

use crate::application::auth::ports::{DeviceCodeError, DeviceCodeFlow, DeviceCodeGrant};
use crate::domain::auth::provider::LoginProvider;
use crate::infrastructure::auth::oauth::{OAuthEndpoints, request_device_code};
use crate::infrastructure::processes::tokio_runtime::build_tokio_runtime;

/// How the adapter builds the runtime a request runs on.
type RuntimeBuilder = fn() -> std::io::Result<tokio::runtime::Runtime>;

#[derive(Debug, Clone)]
pub struct OAuthDeviceCodeFlow {
    endpoints: OAuthEndpoints,
    runtime: RuntimeBuilder,
}

impl OAuthDeviceCodeFlow {
    pub fn new(endpoints: OAuthEndpoints) -> Self {
        Self {
            endpoints,
            runtime: build_tokio_runtime,
        }
    }

    /// This flow on runtimes from `runtime` instead.
    #[cfg(test)]
    fn on_runtimes(endpoints: OAuthEndpoints, runtime: RuntimeBuilder) -> Self {
        Self { endpoints, runtime }
    }
}

impl DeviceCodeFlow for OAuthDeviceCodeFlow {
    fn request(&self, provider: LoginProvider) -> Result<DeviceCodeGrant, DeviceCodeError> {
        // Only a provider with a device-code endpoint offers the flow.
        let Some(config) = self
            .endpoints
            .config_for(provider)
            .filter(|config| !config.device_code_url.is_empty())
        else {
            return Err(DeviceCodeError::Unsupported);
        };
        // Blocking on a runtime from inside another panics: the caller must
        // be outside one.
        match tokio::runtime::Handle::try_current() {
            Err(_outside_a_runtime) => {}
            Ok(_) => {
                return Err(DeviceCodeError::Unavailable(
                    "called from inside an async runtime".to_string(),
                ));
            }
        }
        let runtime =
            (self.runtime)().map_err(|error| DeviceCodeError::Unavailable(error.to_string()))?;
        let response = runtime
            .block_on(request_device_code(&config))
            .map_err(|error| DeviceCodeError::Request(error.to_string()))?;
        Ok(DeviceCodeGrant {
            verification_uri: response.verification_uri,
            user_code: response.user_code,
        })
    }
}

#[cfg(test)]
#[path = "device_code_flow_tests.rs"]
mod tests;
