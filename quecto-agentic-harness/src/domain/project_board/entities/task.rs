use serde::{Deserialize, Serialize};
use super::super::value_objects::status::{Claim, TaskKind, TaskStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub description: String,
    pub status: TaskStatus,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub depends_on: Vec<String>,
    pub claim: Option<Claim>,
    pub pr: Option<u64>,
    pub created: u64,
    pub updated: u64,
}

impl Task {
    pub fn validate(&self) -> Result<(), String> {
        let id_ok = |id: &str| !id.is_empty() && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok(&self.id) { return Err(format!("bad id {:?}", self.id)); }
        if self.title.is_empty() || self.title.len() > 200 || self.title.contains('\n') { return Err("bad title".into()); }
        if self.description.len() > 65536 { return Err("description too long".into()); }
        if !self.children.iter().chain(&self.depends_on).chain(&self.parent).all(|i| id_ok(i)) { return Err("bad ref".into()); }
        let needs_claim = matches!(self.status, TaskStatus::Claimed | TaskStatus::InProgress);
        if needs_claim != self.claim.is_some() && matches!(self.status, TaskStatus::Claimed | TaskStatus::InProgress | TaskStatus::Draft | TaskStatus::Ready) {
            return Err("claim/status mismatch".into());
        }
        Ok(())
    }
}
