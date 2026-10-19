//! Requests and results of the auth use cases.

pub mod device_code;
pub mod login;

pub use device_code::DeviceCodeInstructions;
pub use login::LoginError;
