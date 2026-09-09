use serde::{Deserialize, Serialize};

const MAX_PRESENTATION_BYTES: usize = 256;
const MAX_FOLDER_LABEL_BYTES: usize = 512;

/// Set only by server launch adapters when the agent executes outside the
/// native host context. It is intentionally not a protocol field.
pub const NON_NATIVE_EXECUTION_ENV: &str = "QUECTO_NON_NATIVE_EXECUTION";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionBackend {
    Native,
    NonNative,
}

impl ExecutionBackend {
    /// Interpret process context supplied by the trusted infrastructure layer.
    pub fn from_marker(marker: Option<&std::ffi::OsStr>) -> Self {
        match marker {
            Some(value) if matches!(value.to_str(), Some("1" | "true" | "yes")) => Self::NonNative,
            _ => Self::Native,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderIdentity {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

impl FolderIdentity {
    pub const MAX_ENCODED_BYTES: usize = 4 * 1024;

    pub fn from_unix_bytes(bytes: Vec<u8>) -> Option<Self> {
        Self::encoded_len("unix:", bytes.len()).then_some(Self::Unix(bytes))
    }

    pub fn from_windows_units(units: Vec<u16>) -> Option<Self> {
        Self::encoded_len("windows-u16le:", units.len().saturating_mul(2))
            .then_some(Self::Windows(units))
    }

    fn encoded_len(tag: &str, byte_len: usize) -> bool {
        tag.len().saturating_add(byte_len.saturating_mul(2)) <= Self::MAX_ENCODED_BYTES
    }

    pub fn from_encoded_key(encoded: &str) -> Option<Self> {
        if encoded.len() > Self::MAX_ENCODED_BYTES {
            return None;
        }
        if let Some(payload) = encoded.strip_prefix("unix:") {
            return Self::from_unix_bytes(decode_hex(payload)?);
        }
        let bytes = decode_hex(encoded.strip_prefix("windows-u16le:")?)?;
        if bytes.len() % 2 != 0 {
            return None;
        }
        Self::from_windows_units(
            bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect(),
        )
    }

    pub fn encoded_key(&self) -> String {
        match self {
            Self::Unix(bytes) => format!("unix:{}", encode_hex(bytes)),
            Self::Windows(units) => {
                let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
                format!("windows-u16le:{}", encode_hex(&bytes))
            }
        }
    }

    pub fn unix_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Unix(bytes) => Some(bytes),
            Self::Windows(_) => None,
        }
    }

    pub fn windows_units(&self) -> Option<&[u16]> {
        match self {
            Self::Windows(units) => Some(units),
            Self::Unix(_) => None,
        }
    }
}

impl Serialize for FolderIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.encoded_key())
    }
}

impl<'de> Deserialize<'de> for FolderIdentity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::from_encoded_key(&value)
            .ok_or_else(|| serde::de::Error::custom("invalid or overlong folder identity"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FolderDisplayLabel(String);

impl FolderDisplayLabel {
    pub const MAX_BYTES: usize = MAX_FOLDER_LABEL_BYTES;
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_bounded(&value, Self::MAX_BYTES).then_some(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentDisplayName(String);

impl AgentDisplayName {
    pub const MAX_BYTES: usize = MAX_PRESENTATION_BYTES;
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_bounded(&value, Self::MAX_BYTES).then_some(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GitBranchDisplay(String);

impl GitBranchDisplay {
    pub const MAX_BYTES: usize = MAX_PRESENTATION_BYTES;
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_bounded(&value, Self::MAX_BYTES).then_some(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder_identity: Option<FolderIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder_label: Option<FolderDisplayLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_name: Option<AgentDisplayName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_branch: Option<GitBranchDisplay>,
}

impl<'de> Deserialize<'de> for ExecutionMetadata {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            folder_identity: Option<serde_json::Value>,
            #[serde(default)]
            folder_label: Option<serde_json::Value>,
            #[serde(default)]
            agent_name: Option<serde_json::Value>,
            #[serde(default)]
            git_branch: Option<serde_json::Value>,
        }
        let raw = Raw::deserialize(deserializer)?;
        fn bounded<T>(
            value: Option<serde_json::Value>,
            make: impl FnOnce(String) -> Option<T>,
        ) -> Option<T> {
            value
                .and_then(|value| value.as_str().map(str::to_string))
                .and_then(make)
        }
        Ok(Self {
            folder_identity: raw
                .folder_identity
                .and_then(|value| value.as_str().and_then(FolderIdentity::from_encoded_key)),
            folder_label: bounded(raw.folder_label, FolderDisplayLabel::new),
            agent_name: bounded(raw.agent_name, AgentDisplayName::new),
            git_branch: bounded(raw.git_branch, GitBranchDisplay::new),
        })
    }
}

impl ExecutionMetadata {
    pub fn new(
        folder_identity: Option<FolderIdentity>,
        folder_label: Option<FolderDisplayLabel>,
        agent_name: Option<AgentDisplayName>,
        git_branch: Option<GitBranchDisplay>,
    ) -> Self {
        Self {
            folder_identity,
            folder_label,
            agent_name,
            git_branch,
        }
    }
    pub fn folder_identity(&self) -> Option<&FolderIdentity> {
        self.folder_identity.as_ref()
    }
    pub fn folder_label(&self) -> Option<&FolderDisplayLabel> {
        self.folder_label.as_ref()
    }
    pub fn agent_name(&self) -> Option<&AgentDisplayName> {
        self.agent_name.as_ref()
    }
    pub fn git_branch(&self) -> Option<&GitBranchDisplay> {
        self.git_branch.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMetadataWrite {
    Initialize(ExecutionMetadata),
    UpdateLatest(ExecutionMetadata),
}

fn valid_bounded(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

#[cfg(test)]
mod execution_backend_tests {
    use super::ExecutionBackend;
    use std::ffi::OsStr;

    #[test]
    fn only_the_server_launch_marker_selects_non_native_execution() {
        assert_eq!(
            ExecutionBackend::from_marker(None),
            ExecutionBackend::Native
        );
        assert_eq!(
            ExecutionBackend::from_marker(Some(OsStr::new("0"))),
            ExecutionBackend::Native
        );
        assert_eq!(
            ExecutionBackend::from_marker(Some(OsStr::new("container"))),
            ExecutionBackend::Native
        );
        assert_eq!(
            ExecutionBackend::from_marker(Some(OsStr::new("1"))),
            ExecutionBackend::NonNative
        );
        assert_eq!(
            ExecutionBackend::from_marker(Some(OsStr::new("yes"))),
            ExecutionBackend::NonNative
        );
        assert_eq!(
            ExecutionBackend::from_marker(Some(OsStr::new("true"))),
            ExecutionBackend::NonNative
        );
    }
}
