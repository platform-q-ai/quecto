//! What a device-code login shows its user (#1839).

/// Where to go and which code to enter to authorize the login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeInstructions {
    pub verification_uri: String,
    pub user_code: String,
}
