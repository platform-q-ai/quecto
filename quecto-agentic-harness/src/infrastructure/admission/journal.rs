//! Durable ledger file: temp write, file fsync, rename, directory fsync. Unlike
//! `atomic_write`, a directory sync failure is an error here (ADR-0026).
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::directory::AuthorityDirectory;
use crate::application::ports::{AdmissionJournal, JournalError};
use crate::domain::inference_admission::{
    AdmissionLedger, GroupId, LedgerGroup, OutstandingAttempt,
};

const LEDGER_FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct LedgerWire {
    format: u32,
    epoch: u64,
    outstanding: Vec<OutstandingWire>,
    groups: BTreeMap<String, GroupWire>,
}

#[derive(Serialize, Deserialize)]
struct OutstandingWire {
    group: String,
    scope: u64,
    sequence: u64,
}

#[derive(Serialize, Deserialize)]
struct GroupWire {
    cooldown_remaining_ms: u64,
    pacing_remaining_ms: u64,
    unavailable: bool,
}

fn encode(ledger: &AdmissionLedger) -> Vec<u8> {
    let wire = LedgerWire {
        format: LEDGER_FORMAT,
        epoch: ledger.epoch,
        outstanding: ledger
            .outstanding
            .iter()
            .map(|o| OutstandingWire {
                group: o.group.as_str().to_owned(),
                scope: o.scope,
                sequence: o.sequence,
            })
            .collect(),
        groups: ledger
            .groups
            .iter()
            .map(|(id, g)| {
                (
                    id.as_str().to_owned(),
                    GroupWire {
                        cooldown_remaining_ms: g.cooldown_remaining_ms,
                        pacing_remaining_ms: g.pacing_remaining_ms,
                        unavailable: g.unavailable,
                    },
                )
            })
            .collect(),
    };
    serde_json::to_vec_pretty(&wire).expect("ledger wire is serializable")
}

fn decode(bytes: &[u8]) -> Result<AdmissionLedger, JournalError> {
    let wire: LedgerWire = serde_json::from_slice(bytes)
        .map_err(|e| JournalError::Unavailable(format!("corrupt ledger: {e}")))?;
    if wire.format != LEDGER_FORMAT {
        return Err(JournalError::Unavailable(format!(
            "unsupported ledger format {}",
            wire.format
        )));
    }
    let group = |name: &str| {
        GroupId::new(name).map_err(|e| JournalError::Unavailable(format!("ledger group: {e:?}")))
    };
    let mut outstanding = Vec::with_capacity(wire.outstanding.len());
    for o in &wire.outstanding {
        outstanding.push(OutstandingAttempt {
            group: group(&o.group)?,
            scope: o.scope,
            sequence: o.sequence,
        });
    }
    let mut groups = BTreeMap::new();
    for (name, g) in &wire.groups {
        groups.insert(
            group(name)?,
            LedgerGroup {
                cooldown_remaining_ms: g.cooldown_remaining_ms,
                pacing_remaining_ms: g.pacing_remaining_ms,
                unavailable: g.unavailable,
            },
        );
    }
    Ok(AdmissionLedger {
        epoch: wire.epoch,
        outstanding,
        groups,
    })
}

#[derive(Debug, Clone)]
pub struct FileJournal {
    path: PathBuf,
}

impl FileJournal {
    pub fn new(dir: &AuthorityDirectory) -> Self {
        Self {
            path: dir.journal_path(),
        }
    }

    /// `None` only when no ledger file exists. Any unreadable or corrupt file is
    /// an error: an authority must never restart with an empty ledger by accident.
    pub fn load(dir: &AuthorityDirectory) -> Result<Option<AdmissionLedger>, JournalError> {
        match fs::read(dir.journal_path()) {
            Ok(bytes) => decode(&bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(JournalError::Unavailable(format!("read ledger: {e}"))),
        }
    }

    fn write_durably(&self, bytes: &[u8]) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "journal path has no parent")
        })?;
        let temp = parent.join(format!(".journal.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            fs::File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

impl AdmissionJournal for FileJournal {
    fn persist(&mut self, ledger: &AdmissionLedger) -> Result<(), JournalError> {
        self.write_durably(&encode(ledger))
            .map_err(|e| JournalError::Unavailable(format!("persist ledger: {e}")))
    }
}
