//! Start a device-code login (#1839): `auth login --provider P
//! --device-code` asks `P` for a device-code grant and shows its user where
//! to authorize. Nothing is stored: the grant is never polled for a token.

use std::sync::Arc;

use crate::application::auth::dto::{DeviceCodeInstructions, LoginError};
use crate::application::auth::ports::{DeviceCodeError, DeviceCodeFlow};
use crate::domain::auth::provider::LoginProvider;

pub struct StartDeviceCodeLogin {
    flow: Arc<dyn DeviceCodeFlow>,
}

impl StartDeviceCodeLogin {
    pub fn new(flow: Arc<dyn DeviceCodeFlow>) -> Self {
        Self { flow }
    }

    /// Request `provider`'s grant. A provider outside the login allowlist,
    /// or one not offering a device-code login, is refused without a
    /// request.
    pub fn execute(&self, provider: &str) -> Result<DeviceCodeInstructions, LoginError> {
        let Some(login) = LoginProvider::parse(provider) else {
            return Err(LoginError::UnknownProvider(provider.to_string()));
        };
        if !login.offers_device_code() {
            return Err(LoginError::DeviceCodeUnsupported(login));
        }
        match self.flow.request(login) {
            Ok(grant) => Ok(DeviceCodeInstructions {
                verification_uri: grant.verification_uri,
                user_code: grant.user_code,
            }),
            Err(DeviceCodeError::Unsupported) => Err(LoginError::DeviceCodeUnsupported(login)),
            Err(DeviceCodeError::Unavailable(reason)) => Err(LoginError::Unavailable(reason)),
            Err(DeviceCodeError::Request(reason)) => Err(LoginError::DeviceCodeRequest(reason)),
        }
    }
}

impl std::fmt::Debug for StartDeviceCodeLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartDeviceCodeLogin")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "start_device_code_login_tests.rs"]
mod tests;
