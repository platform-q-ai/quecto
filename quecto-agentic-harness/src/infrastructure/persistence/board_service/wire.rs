//! SPIKE (board service): the JSON the server and its clients exchange.
//! Artifacts travel as their markdown files; nothing is encoded twice.
use super::files::BoardFile;
use crate::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardStoreError, BoardWrite, KeepCopy, SetAsideClear,
};
use crate::domain::project_board::entities::claim::Identity;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireFile {
    pub path: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64: Option<String>,
}

impl WireFile {
    pub fn of(file: &BoardFile) -> Self {
        match String::from_utf8(file.bytes.clone()) {
            Ok(text) => Self {
                path: file.path.clone(),
                mode: file.mode.clone(),
                text: Some(text),
                b64: None,
            },
            Err(_) => Self {
                path: file.path.clone(),
                mode: file.mode.clone(),
                text: None,
                b64: Some(STANDARD.encode(&file.bytes)),
            },
        }
    }

    pub fn file(self) -> Result<BoardFile, String> {
        let bytes = match (self.text, self.b64) {
            (Some(text), None) => text.into_bytes(),
            (None, Some(b64)) => STANDARD.decode(b64).map_err(|error| error.to_string())?,
            _ => return Err(format!("{}: a file is text or base64", self.path)),
        };
        Ok(BoardFile {
            path: self.path,
            mode: self.mode,
            bytes,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireIdentity {
    pub name: String,
    pub email: String,
}

impl From<&Identity> for WireIdentity {
    fn from(identity: &Identity) -> Self {
        Self {
            name: identity.name.clone(),
            email: identity.email.clone(),
        }
    }
}

impl From<WireIdentity> for Identity {
    fn from(identity: WireIdentity) -> Self {
        Self {
            name: identity.name,
            email: identity.email,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotWire {
    pub head: String,
    pub files: Vec<WireFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteWire {
    pub expected: String,
    pub files: Vec<WireFile>,
    pub message: String,
    pub writer: WireIdentity,
    pub write_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepWire {
    Keep {
        keep: String,
        seen: BTreeSet<String>,
    },
    Drop {
        seen: BTreeSet<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearWire {
    pub expected: String,
    pub path: String,
    pub object: String,
    pub repeated: BTreeMap<String, KeepWire>,
    pub message: String,
    pub writer: WireIdentity,
    pub write_id: String,
}

impl ClearWire {
    pub fn clear(&self) -> SetAsideClear {
        SetAsideClear {
            path: self.path.clone(),
            object: self.object.clone(),
            repeated: self
                .repeated
                .iter()
                .map(|(path, keep)| {
                    let keep = match keep {
                        KeepWire::Keep { keep, seen } => KeepCopy::Object {
                            keep: keep.clone(),
                            seen: seen.clone(),
                        },
                        KeepWire::Drop { seen } => KeepCopy::Drop(seen.clone()),
                    };
                    (path.clone(), keep)
                })
                .collect(),
        }
    }

    pub fn repeated_of(clear: &SetAsideClear) -> BTreeMap<String, KeepWire> {
        clear
            .repeated
            .iter()
            .map(|(path, keep)| {
                let keep = match keep {
                    KeepCopy::Object { keep, seen } => KeepWire::Keep {
                        keep: keep.clone(),
                        seen: seen.clone(),
                    },
                    KeepCopy::Drop(seen) => KeepWire::Drop { seen: seen.clone() },
                };
                (path.clone(), keep)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapWire {
    pub writer: WireIdentity,
}

/// Every answer of a write, a clear or a bootstrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum OutcomeWire {
    Written { head: String },
    Lost,
    Refused { reason: String },
    Created { head: String },
    Existing { head: String },
    Error(ErrorWire),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorWire {
    pub kind: String,
    pub why: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub object: Option<String>,
}

impl ErrorWire {
    pub fn of(error: &BoardStoreError) -> Self {
        let (kind, why) = match error {
            BoardStoreError::NoBoard => ("no_board", String::new()),
            BoardStoreError::Clash(why) => ("clash", why.clone()),
            BoardStoreError::InvalidWrite(why) => ("invalid_write", why.clone()),
            BoardStoreError::WriteTooLarge(why) => ("write_too_large", why.clone()),
            BoardStoreError::Unavailable(why) => ("unavailable", why.clone()),
            BoardStoreError::TooLarge(why) => ("too_large", why.clone()),
            BoardStoreError::FolderSetAside { why, .. } => ("folder_set_aside", why.clone()),
        };
        let (path, object) = match error {
            BoardStoreError::FolderSetAside { path, object, .. } => {
                (Some(path.clone()), Some(object.clone()))
            }
            _ => (None, None),
        };
        Self {
            kind: kind.into(),
            why,
            path,
            object,
        }
    }

    pub fn error(self) -> BoardStoreError {
        match self.kind.as_str() {
            "no_board" => BoardStoreError::NoBoard,
            "clash" => BoardStoreError::Clash(self.why),
            "invalid_write" => BoardStoreError::InvalidWrite(self.why),
            "write_too_large" => BoardStoreError::WriteTooLarge(self.why),
            "too_large" => BoardStoreError::TooLarge(self.why),
            "folder_set_aside" => BoardStoreError::FolderSetAside {
                why: self.why,
                path: self.path.unwrap_or_default(),
                object: self.object.unwrap_or_default(),
            },
            _ => BoardStoreError::Unavailable(self.why),
        }
    }
}

impl OutcomeWire {
    pub fn of_write(result: &Result<BoardWrite, BoardStoreError>) -> Self {
        match result {
            Ok(BoardWrite::Written(head)) => Self::Written {
                head: head.0.clone(),
            },
            Ok(BoardWrite::Lost) => Self::Lost,
            Ok(BoardWrite::Refused(reason)) => Self::Refused {
                reason: reason.clone(),
            },
            Err(error) => Self::Error(ErrorWire::of(error)),
        }
    }

    pub fn of_bootstrap(result: &Result<BoardBootstrap, BoardStoreError>) -> Self {
        match result {
            Ok(BoardBootstrap::Created(head)) => Self::Created {
                head: head.0.clone(),
            },
            Ok(BoardBootstrap::Existing(head)) => Self::Existing {
                head: head.0.clone(),
            },
            Err(error) => Self::Error(ErrorWire::of(error)),
        }
    }

    pub fn write(self) -> Result<BoardWrite, BoardStoreError> {
        match self {
            Self::Written { head } => Ok(BoardWrite::Written(BoardHead(head))),
            Self::Lost => Ok(BoardWrite::Lost),
            Self::Refused { reason } => Ok(BoardWrite::Refused(reason)),
            Self::Error(error) => Err(error.error()),
            other => Err(BoardStoreError::Unavailable(format!(
                "unexpected {other:?}"
            ))),
        }
    }

    pub fn bootstrap(self) -> Result<BoardBootstrap, BoardStoreError> {
        match self {
            Self::Created { head } => Ok(BoardBootstrap::Created(BoardHead(head))),
            Self::Existing { head } => Ok(BoardBootstrap::Existing(BoardHead(head))),
            Self::Error(error) => Err(error.error()),
            other => Err(BoardStoreError::Unavailable(format!(
                "unexpected {other:?}"
            ))),
        }
    }
}

/// One revision on the change feed: no artifact text, only which files
/// changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedEvent {
    pub epoch: u64,
    pub revision: u64,
    pub kind: String,
    pub writer: String,
    pub message: String,
    pub server_time: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusWire {
    pub role: String,
    pub head: Option<String>,
    pub lease: Option<String>,
    pub server_time: String,
}
