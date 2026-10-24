//! Real deterministic Git serialization and captured-lease publication.
use super::git::{Git, GitEnvironment};
use crate::application::project_board::ports::history::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
fn malformed(why: impl ToString) -> HistoryError {
    HistoryError::Malformed(why.to_string())
}
fn unavailable(why: impl ToString) -> HistoryError {
    HistoryError::Unavailable(why.to_string())
}
fn bounds(why: &str) -> HistoryError {
    HistoryError::Bounds(why.into())
}
fn git(repo: &std::path::Path) -> HistoryResult<Git> {
    Git::reader(repo, GitEnvironment::of_process())
        .map_err(|e| HistoryError::Unavailable(e.to_string()))
}
fn kind(k: HistoryKind) -> &'static str {
    match k {
        HistoryKind::Blob => "blob",
        HistoryKind::Tree => "tree",
        HistoryKind::Commit => "commit",
    }
}
fn oid(id: &str, width: usize) -> HistoryResult<()> {
    match id.len() == width
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        true => Ok(()),
        false => Err(malformed("invalid object name")),
    }
}
fn charge(value: &mut usize, n: usize, cap: usize) -> HistoryResult<()> {
    *value = value
        .checked_add(n)
        .ok_or_else(|| bounds("counter overflow"))?;
    match *value <= cap {
        true => Ok(()),
        false => Err(bounds("history admission exceeded")),
    }
}
pub struct GitBoardHistorySink {
    repo: PathBuf,
    target_ref: String,
}
impl GitBoardHistorySink {
    pub fn new(repo: PathBuf, target_ref: String) -> Self {
        Self { repo, target_ref }
    }
}
impl BoardHistorySink for GitBoardHistorySink {
    fn raw(
        &self,
        k: HistoryKind,
        bytes: &[u8],
        expected: Option<&str>,
        dry: bool,
    ) -> HistoryResult<String> {
        let cap = match k {
            HistoryKind::Blob => 64 * 1024 * 1024,
            HistoryKind::Tree => 655360,
            HistoryKind::Commit => 65536,
        };
        if bytes.len() <= cap {
        } else {
            return Err(bounds("sink object size"));
        }
        let git = git(&self.repo)?;
        let args = if dry {
            vec!["hash-object", "--literally", "-t", kind(k), "--stdin"]
        } else {
            vec!["hash-object", "--literally", "-w", "-t", kind(k), "--stdin"]
        };
        // Hash first, before any write, including expected-OID verification.
        let hash = git
            .text(
                &["hash-object", "--literally", "-t", kind(k), "--stdin"],
                Some(bytes),
            )
            .map_err(unavailable)?;
        if expected.is_none_or(|expected| expected == hash) {
        } else {
            return Err(malformed("expected raw object hash mismatch"));
        }
        if dry {
        } else {
            let written = git.text(&args, Some(bytes)).map_err(unavailable)?;
            if written == hash {
            } else {
                return Err(malformed("written hash mismatch"));
            }
        }
        Ok(hash)
    }
    fn tree(&self, entries: &[HistoryEntry], dry: bool) -> HistoryResult<String> {
        let git = git(&self.repo)?;
        let width = match git
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(malformed)?
            .as_str()
        {
            "sha1" => 40,
            "sha256" => 64,
            _ => return Err(malformed("format")),
        };
        if entries.len() <= 65536 {
        } else {
            return Err(bounds("native tree entry count"));
        }
        let mut size = 0usize;
        for e in entries {
            if ["100644", "100755", "120000", "040000", "160000"].contains(&e.mode.as_str())
                && (1..=4096).contains(&e.name.len())
                && e.name
                    .iter()
                    .all(|b| (1..=46).contains(b) || (48..=255).contains(b))
            {
            } else {
                return Err(malformed("native tree entry"));
            }
            oid(&e.oid, width)?;
            charge(
                &mut size,
                e.mode.trim_start_matches('0').len() + 1 + e.name.len() + 1 + width / 2,
                655360,
            )?;
        }
        let mut names = BTreeSet::new();
        if entries.iter().all(|e| names.insert(e.name.as_slice())) {
        } else {
            return Err(malformed("duplicate native tree name"));
        }
        let mut entries: Vec<_> = entries.iter().collect();
        entries.sort_by(|a, b| {
            a.name
                .iter()
                .copied()
                .chain((a.mode == "040000").then_some(b'/'))
                .cmp(
                    b.name
                        .iter()
                        .copied()
                        .chain((b.mode == "040000").then_some(b'/')),
                )
        });
        let mut bytes = Vec::with_capacity(size);
        for e in entries {
            bytes.extend_from_slice(e.mode.trim_start_matches('0').as_bytes());
            bytes.push(b' ');
            bytes.extend_from_slice(&e.name);
            bytes.push(0);
            for i in (0..width).step_by(2) {
                bytes.push(u8::from_str_radix(&e.oid[i..i + 2], 16).map_err(malformed)?);
            }
        }
        self.raw(HistoryKind::Tree, &bytes, None, dry)
    }
    fn commit(&self, revision: HistoryCommit<'_>, dry: bool) -> HistoryResult<String> {
        let HistoryCommit {
            tree,
            parent,
            writer,
            message,
            at,
        } = revision;
        writer.validate("history writer").map_err(malformed)?;
        if [40, 64].contains(&tree.len()) {
        } else {
            return Err(malformed("commit tree width"));
        }
        oid(tree, tree.len())?;
        if let Some(parent) = parent {
            oid(parent, tree.len())?;
        }
        if at >= 0 && message.len() <= 65536 && message.bytes().all(|b| (1..=255).contains(&b)) {
        } else {
            return Err(malformed("commit metadata"));
        }
        let time = at.to_string();
        let identity = writer
            .name
            .len()
            .checked_add(writer.email.len())
            .and_then(|n| n.checked_add(time.len()))
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| bounds("commit metadata overflow"))?;
        let size = (6 + tree.len())
            .checked_add(parent.map_or(0, |p| 8 + p.len()))
            .and_then(|n| n.checked_add(39 + identity + 2))
            .and_then(|n| n.checked_add(message.len()))
            .ok_or_else(|| bounds("commit size overflow"))?;
        if size <= 65536 {
        } else {
            return Err(bounds("commit size"));
        }
        let mut body = String::with_capacity(size);
        body.push_str(&format!("tree {tree}\n"));
        if let Some(parent) = parent {
            body.push_str(&format!("parent {parent}\n"));
        }
        body.push_str(&format!(
            "author {} <{}> {at} +0000\ncommitter {} <{}> {at} +0000\n\n{message}\n",
            writer.name, writer.email, writer.name, writer.email
        ));
        assert_eq!(body.len(), size, "checked commit framing size");
        self.raw(HistoryKind::Commit, body.as_bytes(), None, dry)
    }
    fn publish(&self, expected: Option<&str>, new: &str) -> HistoryResult<()> {
        let git = git(&self.repo)?;
        if self.target_ref.len() <= 1024
            && self.target_ref.starts_with("refs/heads/")
            && git
                .local(&["check-ref-format", &self.target_ref], None, 1024)
                .map_err(malformed)?
                .status
                .success()
        {
        } else {
            return Err(malformed("destination ref"));
        }
        let width = match git
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(malformed)?
            .as_str()
        {
            "sha1" => 40,
            "sha256" => 64,
            _ => return Err(malformed("destination format")),
        };
        oid(new, width)?;
        if let Some(old) = expected {
            oid(old, width)?;
        }
        let absent = "0".repeat(width);
        let result = git.local(
            &[
                "update-ref",
                "--no-deref",
                &self.target_ref,
                new,
                expected.unwrap_or(&absent),
            ],
            None,
            65536,
        );
        match result {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => {
                let failure = super::git::failed(
                    &[
                        "update-ref",
                        "--no-deref",
                        &self.target_ref,
                        new,
                        expected.unwrap_or(&absent),
                    ],
                    &out,
                );
                let detail = format!(
                    "expected={expected:?} new={new}; verified objects may remain unreachable: {failure}"
                );
                let observed = git.local(
                    &["rev-parse", "--verify", "--quiet", &self.target_ref],
                    None,
                    1024,
                );
                let observed = match observed {
                    Ok(out) if out.status.success() => {
                        let id = std::str::from_utf8(&out.stdout)
                            .map_err(|_| HistoryError::PublicationUnknown(detail.clone()))?
                            .trim();
                        oid(id, width)
                            .map_err(|_| HistoryError::PublicationUnknown(detail.clone()))?;
                        Some(id.to_owned())
                    }
                    Ok(out) if out.status.code() == Some(1) => None,
                    _ => return Err(HistoryError::PublicationUnknown(detail)),
                };
                if observed.as_deref() == Some(new) {
                    Err(HistoryError::PublicationUnknown(detail))
                } else if observed.as_deref() == expected {
                    Err(HistoryError::Unavailable(detail))
                } else {
                    Err(HistoryError::Conflict(format!(
                        "{detail}; observed={observed:?}"
                    )))
                }
            }
            Err(e) => Err(HistoryError::PublicationUnknown(format!(
                "expected={expected:?} new={new}: {e}"
            ))),
        }
    }
    fn finish_dry_run(&self) -> HistoryResult<()> {
        Ok(())
    }
}

// RED-only callable seam; no Git capture or successful acceptance behavior.
pub struct GitBoardHistorySource;
impl GitBoardHistorySource {
    pub fn new(_: PathBuf, _: String) -> Self {
        Self
    }
}
impl BoardHistorySource for GitBoardHistorySource {
    fn capture(&self) -> HistoryResult<LinearHistory> {
        Err(HistoryError::Unavailable(
            "B source capture not implemented".into(),
        ))
    }
}
