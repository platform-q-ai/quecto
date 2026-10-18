//! The device-code flow over the providers' OAuth endpoints (#1839): the
//! application's `DeviceCodeFlow`, run on a short-lived current-thread
//! runtime so the CLI's per-command call needs no ambient one.

use crate::application::auth::ports::{DeviceCodeError, DeviceCodeFlow, DeviceCodeGrant};
use crate::domain::auth::provider::LoginProvider;
use crate::infrastructure::auth::oauth::{OAuthEndpoints, request_device_code};

#[derive(Debug, Clone)]
pub struct OAuthDeviceCodeFlow {
    endpoints: OAuthEndpoints,
}

impl OAuthDeviceCodeFlow {
    pub fn new(endpoints: OAuthEndpoints) -> Self {
        Self { endpoints }
    }
}

impl DeviceCodeFlow for OAuthDeviceCodeFlow {
    fn request(&self, provider: LoginProvider) -> Result<DeviceCodeGrant, DeviceCodeError> {
        // Only a provider with a device-code endpoint offers the flow: xAI
        // and Anthropic deliberately have none (PR #1087 review).
        let Some(config) = self
            .endpoints
            .config_for(provider)
            .filter(|config| !config.device_code_url.is_empty())
        else {
            return Err(DeviceCodeError::Unsupported);
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| DeviceCodeError::Runtime(error.to_string()))?;
        let response = runtime
            .block_on(request_device_code(&config))
            .map_err(|error| DeviceCodeError::Request(error.to_string()))?;
        Ok(DeviceCodeGrant {
            verification_uri: response.verification_uri,
            user_code: response.user_code,
        })
    }
}
