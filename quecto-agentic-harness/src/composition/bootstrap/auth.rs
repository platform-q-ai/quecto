//! The auth use cases for one run (#1839): the login use cases over the
//! base directory's `credentials.json` and the providers' OAuth endpoints
//! (or the one test server a run points them all at). `main` hands
//! [`build_auth_handles`] to the CLI; `quecto auth` invokes the handles.

use std::sync::Arc;

use crate::application::auth::ports::{CredentialStorePort, DeviceCodeFlow};
use crate::application::auth::use_cases::{LogInWithToken, StartDeviceCodeLogin};
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::auth::device_code_flow::OAuthDeviceCodeFlow;
use crate::infrastructure::auth::oauth::OAuthEndpoints;
use crate::interface::cli::auth::{AuthEnvironment, AuthHandles};

pub fn build_auth_handles(environment: &AuthEnvironment) -> AuthHandles {
    let store: Arc<dyn CredentialStorePort> = Arc::new(CredentialStore::new(&environment.base_dir));
    let endpoints = OAuthEndpoints::new(environment.oauth_base_url.clone());
    let device_code: Arc<dyn DeviceCodeFlow> = Arc::new(OAuthDeviceCodeFlow::new(endpoints));
    AuthHandles {
        token_login: Arc::new(LogInWithToken::new(store)),
        device_code_login: Arc::new(StartDeviceCodeLogin::new(device_code)),
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
