//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! file reservations (#2272, #2275), and a lexical
//! stand-in for the checkout. A task id matches by the rough INTEGER
//! affinity of `board_test_support_tasks`; the SQLite adapter's contract
//! tests pin the real binding.
use std::collections::BTreeMap;

use serde_json::Value;

use super::MemoryTransaction;
use super::tasks::{StoredFile, affinity};
use crate::application::swarm::dto::{FileRow, NewReservation};
use crate::application::swarm::ports::{BoardFiles, CheckoutPaths};
use crate::domain::swarm::{BoardError, RefusalKind};

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl BoardFiles for MemoryTransaction<'_> {
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError> {
        self.note(format!("delete_claim_files {task}"));
        let task = affinity(task);
        self.state
            .borrow_mut()
            .files
            .retain(|file| !(Some(file.task) == task && claim.as_str() == Some(&file.claim)));
        Ok(())
    }

    fn file_count(&self) -> Result<i64, BoardError> {
        Ok(i64::try_from(self.state.borrow().files.len()).unwrap())
    }

    fn file_reserved(&self, path: &str) -> Result<bool, BoardError> {
        self.note(format!("file_reserved {path}"));
        Ok(self
            .state
            .borrow()
            .files
            .iter()
            .any(|file| file.path == path))
    }

    fn insert_files(&self, reservation: &NewReservation) -> Result<(), BoardError> {
        self.note(format!("insert_files {:?}", reservation.paths));
        let task = affinity(&reservation.task).expect("an owned task's id");
        let claim = text_of(&reservation.claim).expect("an owned claim's token");
        let mut state = self.state.borrow_mut();
        for path in &reservation.paths {
            assert!(
                state.files.iter().all(|file| &file.path != path),
                "files.path is the primary key: {path}"
            );
            state.files.push(StoredFile {
                path: path.clone(),
                task,
                owner: reservation.owner.clone(),
                claim: claim.clone(),
                token: reservation.token.clone(),
            });
        }
        Ok(())
    }

    fn delete_reservation(
        &self,
        task: &Value,
        owner: &str,
        claim: &Value,
        token: &Value,
    ) -> Result<usize, BoardError> {
        self.note(format!("delete_reservation {task} {token}"));
        let task = affinity(task);
        let (claim, token) = (text_of(claim), text_of(token));
        let files = &mut self.state.borrow_mut().files;
        let before = files.len();
        files.retain(|file| {
            !(Some(file.task) == task
                && file.owner == owner
                && claim.as_deref() == Some(&file.claim)
                && token.as_deref() == Some(&file.token))
        });
        Ok(before - files.len())
    }

    fn task_file_count(&self, task: &Value) -> Result<i64, BoardError> {
        let task = affinity(task);
        let files = &self.state.borrow().files;
        Ok(i64::try_from(files.iter().filter(|file| Some(file.task) == task).count()).unwrap())
    }

    fn delete_task_files(&self, task: &Value) -> Result<(), BoardError> {
        self.note(format!("delete_task_files {task}"));
        let task = affinity(task);
        self.state
            .borrow_mut()
            .files
            .retain(|file| Some(file.task) != task);
        Ok(())
    }

    fn owner_file_count(&self, owner: &Value) -> Result<i64, BoardError> {
        let files = &self.state.borrow().files;
        let owned = files
            .iter()
            .filter(|file| owner.as_str() == Some(&file.owner));
        Ok(i64::try_from(owned.count()).unwrap())
    }

    fn delete_owner_files(&self, owner: &Value) -> Result<(), BoardError> {
        self.note(format!("delete_owner_files {owner}"));
        self.state
            .borrow_mut()
            .files
            .retain(|file| owner.as_str() != Some(&file.owner));
        Ok(())
    }

    fn file_page(&self, offset: u64, limit: i64) -> Result<Vec<FileRow>, BoardError> {
        let mut files = self.state.borrow().files.clone();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let (offset, limit) = (
            usize::try_from(offset).unwrap(),
            usize::try_from(limit).unwrap(),
        );
        Ok(files
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|file| FileRow {
                columns: vec![
                    ("path".to_owned(), Value::from(file.path)),
                    ("task".to_owned(), Value::from(file.task)),
                    ("owner".to_owned(), Value::from(file.owner)),
                    ("claim".to_owned(), Value::from(file.claim)),
                    ("token".to_owned(), Value::from(file.token)),
                ],
            })
            .collect())
    }
}

/// Paths normalised lexically (`.` dropped, `..` taken back), with the
/// symlinked directories `aliases` names followed first; one escaping the
/// root is refused with the board's text. The SQLite-free stand-in for
/// the filesystem adapter, whose own tests pin Python's `resolve()`.
#[derive(Default)]
pub struct LexicalCheckout {
    pub aliases: BTreeMap<String, String>,
}

impl CheckoutPaths for LexicalCheckout {
    fn normalize(&self, path: &str) -> Result<String, BoardError> {
        let escape = || {
            BoardError::new(
                RefusalKind::Invalid,
                "file must resolve inside the shared checkout",
            )
        };
        let mut parts: Vec<String> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop().ok_or_else(escape)?;
                }
                name => {
                    parts.push(name.to_owned());
                    let joined = parts.join("/");
                    if let Some(target) = self.aliases.get(&joined) {
                        parts = target.split('/').map(str::to_owned).collect();
                    }
                }
            }
        }
        if path.starts_with('/') {
            return Err(escape());
        }
        Ok(if parts.is_empty() {
            ".".to_owned()
        } else {
            parts.join("/")
        })
    }
}
