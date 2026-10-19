//! The auth use cases for one run (#1839, #1841, #1842): the login, logout
//! and status use cases over the base directory's `credentials.json`, the system clock and
//! the providers' OAuth endpoints (or the one test server a run points them
//! all at). `main` hands [`build_auth_handles`] to the CLI; `quecto auth`
//! invokes the handles.

use std::sync::Arc;

use crate::application::auth::ports::{AuthClock, CredentialStorePort, DeviceCodeFlow};
use crate::application::auth::use_cases::{
    InspectAuthStatus, LogInWithOAuth, LogInWithToken, RemoveStoredCredentials,
    StartDeviceCodeLogin,
};
use crate::infrastructure::auth::browser_login::OAuthBrowserLogin;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::auth::device_code_flow::OAuthDeviceCodeFlow;
use crate::infrastructure::auth::oauth::OAuthEndpoints;
use crate::infrastructure::time::auth_clock::SystemAuthClock;
use crate::interface::cli::auth::{AuthEnvironment, AuthHandles};

pub fn build_auth_handles(environment: &AuthEnvironment) -> AuthHandles {
    let store: Arc<dyn CredentialStorePort> = Arc::new(CredentialStore::new(&environment.base_dir));
    let endpoints = OAuthEndpoints::new(environment.oauth_base_url.clone());
    let device_code: Arc<dyn DeviceCodeFlow> =
        Arc::new(OAuthDeviceCodeFlow::new(endpoints.clone()));
    let browser = Arc::new(OAuthBrowserLogin::new(endpoints));
    let clock: Arc<dyn AuthClock> = Arc::new(SystemAuthClock);
    AuthHandles {
        token_login: Arc::new(LogInWithToken::new(Arc::clone(&store))),
        logout: Arc::new(RemoveStoredCredentials::new(Arc::clone(&store))),
        status: Arc::new(InspectAuthStatus::new(
            Arc::clone(&store),
            Arc::clone(&clock),
        )),
        device_code_login: Arc::new(StartDeviceCodeLogin::new(device_code)),
        oauth_login: Arc::new(LogInWithOAuth::new(
            Arc::clone(&browser) as _,
            Arc::clone(&browser) as _,
            browser,
            store,
            clock,
        )),
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
