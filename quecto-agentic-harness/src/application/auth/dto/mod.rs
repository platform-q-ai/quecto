//! Requests and results of the auth use cases.

pub mod device_code;
pub mod login;
pub mod oauth;

pub use device_code::DeviceCodeInstructions;
pub use login::LoginError;
pub use oauth::{AuthorizationCode, CallbackFailure, OAuthLogin, OAuthLoginAttempt};
