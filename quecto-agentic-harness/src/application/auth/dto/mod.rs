//! Requests and results of the auth use cases.

pub mod device_code;
pub mod login;
pub mod oauth;
pub mod removal;
pub mod status;

pub use device_code::DeviceCodeInstructions;
pub use login::LoginError;
pub use oauth::{AuthorizationCode, CallbackFailure, OAuthLogin, OAuthLoginAttempt};
pub use removal::{Removal, RemoveError};
pub use status::{AuthStatusError, ProviderAuthStatus};

/// How a started login's code comes back: part of the attempt the
/// interface presents, so it is offered here beside the attempt.
pub use crate::application::auth::ports::AuthorizationReturn;
