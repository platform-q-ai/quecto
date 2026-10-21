//! Carrying a submission's evidence forward to the coordinator's commit
//! (#2687). A worker may not commit, so it submits evidence at the commit
//! its uncommitted change sits on; the coordinator, the evidence gate,
//! commits that change and verifies at its own commit. The submission's
//! evidence may then be carried to that commit, so the worker is not sent
//! through release and reclaim to resubmit the same evidence.
//!
//! The policy here is what is carried, over values only: every entry's
//! revision is a full commit id (40 or 64 lowercase hex digits), and the
//! carried evidence is the submission's, entry for entry, each revision
//! set to the commit verified. Whether each earlier commit is in that
//! commit's history is the application's to ask of the checkout.
use serde_json::Value;

/// What a verification at a later commit carries: the earlier commits the
/// submission's evidence names (each once, in order of first mention) and
/// the evidence at the verified commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceCarry {
    pub sources: Vec<String>,
    pub evidence: Value,
}

/// Whether `text` is a full git commit id: 40 (SHA-1) or 64 (SHA-256)
/// lowercase hex digits. A branch name, an abbreviated id or any other
/// text is not: evidence is carried only between commits named exactly.
pub fn full_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The carry of `evidence` (a submission's, as stored) to `revision`, when
/// both name full commit ids: `revision` is one, the evidence is a
/// nonempty list of objects, and each object's `revision` is one. Answers
/// `None` for anything else, and when no entry names another commit (there
/// is then nothing to carry).
pub fn evidence_carry(evidence: &Value, revision: &Value) -> Option<EvidenceCarry> {
    let (Value::String(target), Value::Array(entries)) = (revision, evidence) else {
        return None;
    };
    let (true, false) = (full_commit_id(target), entries.is_empty()) else {
        return None;
    };
    let mut sources: Vec<String> = Vec::new();
    let mut carried = Vec::with_capacity(entries.len());
    for entry in entries {
        let Value::Object(fields) = entry else {
            return None;
        };
        let Some(Value::String(source)) = fields.get("revision") else {
            return None;
        };
        if !full_commit_id(source) {
            return None;
        }
        let earlier = source != target && !sources.contains(source);
        if earlier {
            sources.push(source.clone());
        }
        let mut fields = fields.clone();
        fields.insert("revision".to_owned(), Value::String(target.clone()));
        carried.push(Value::Object(fields));
    }
    match sources.is_empty() {
        true => None,
        false => Some(EvidenceCarry {
            sources,
            evidence: Value::Array(carried),
        }),
    }
}

#[cfg(test)]
#[path = "evidence_carry_tests.rs"]
mod tests;
