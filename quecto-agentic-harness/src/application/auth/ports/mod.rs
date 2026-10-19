//! The effects the auth use cases need, implemented in
//! `infrastructure::auth` and `infrastructure::time`.

pub mod authorization_callback;
pub mod browser_authorization;
pub mod clock;
pub mod credential_store;
pub mod device_code_flow;
pub mod token_exchange;

pub use authorization_callback::{AuthorizationCallback, AuthorizationCallbackError};
pub use browser_authorization::{
    AuthorizationReturn, BrowserAuthorization, BrowserAuthorizationError, PendingAuthorization,
};
pub use clock::AuthClock;
pub use credential_store::{CredentialStoreError, CredentialStorePort};
pub use device_code_flow::{DeviceCodeError, DeviceCodeFlow, DeviceCodeGrant};
pub use token_exchange::{IssuedToken, TokenExchange, TokenExchangeError};
