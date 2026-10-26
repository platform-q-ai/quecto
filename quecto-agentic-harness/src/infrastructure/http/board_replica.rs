//! Replication over a board server (epic #2705 B11): a primary's batch as
//! JSON on the wire (`GET /v1/boards/{id}/replicate?after=&objects=
//! &limit=`), each field read strictly (an unknown one refused), bytes in
//! base64.
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::application::project_board::ports::TokenKind;
use crate::application::project_board::ports::replication::{
    AdminToken, ReplicaBatch, ReplicaContent, ReplicaEntry, ReplicaEpoch, ReplicaMeta,
    ReplicaObject, ReplicaRevision,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    after: u64,
    chain_after: Option<String>,
    objects: Vec<Object>,
    revisions: Vec<Revision>,
    epochs: Vec<Epoch>,
    meta: Option<Meta>,
    tokens: Vec<Token>,
    tip: u64,
    more: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    sequence: u64,
    oid: String,
    /// A blob's bytes; none for a tree.
    blob: Option<String>,
    /// A tree's size and entries; none for a blob.
    tree: Option<(u64, Vec<Entry>)>,
    /// An imported original: its raw tree (none for a blob); absent if native.
    original: Option<Original>,
}

/// A tree's entry: its mode, its name in base64, and its object's id.
type Entry = (String, String, String);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Original {
    raw: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    head: String,
    root: String,
    name: String,
    email: String,
    revision: u64,
    parent: Option<u64>,
    message: String,
    at: i64,
    git_commit: Option<String>,
    git_meta: Option<String>,
    principal: Option<String>,
    chain: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Epoch {
    epoch: u64,
    from_revision: u64,
    reason: String,
    at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    board_id: Option<String>,
    algorithm: String,
    source_remote: Option<String>,
    source_ref: Option<String>,
    source_head: Option<String>,
    imported_count: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    name: String,
    created: String,
    /// `device` or `replica`, an allowlist.
    kind: String,
    sha256: String,
}

fn coded(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn decoded(text: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(text)
        .map_err(|error| format!("bytes not in base64: {error}"))
}

/// `batch` as the wire's JSON.
///
/// # Errors
/// It could not be written (it holds nothing that cannot be).
pub fn encode_batch(batch: &ReplicaBatch) -> Result<Vec<u8>, String> {
    let objects = batch.objects.iter().map(|object| {
        let (blob, tree) = match &object.content {
            ReplicaContent::Blob(bytes) => (Some(coded(bytes)), None),
            ReplicaContent::Tree { size, entries } => {
                let entries = entries.iter().map(|entry| {
                    (
                        entry.mode.clone(),
                        coded(&entry.raw_name),
                        entry.oid.clone(),
                    )
                });
                (None, Some((*size, entries.collect())))
            }
        };
        Object {
            sequence: object.sequence,
            oid: object.oid.clone(),
            blob,
            tree,
            original: object.original.as_ref().map(|raw| Original {
                raw: raw.as_deref().map(coded),
            }),
        }
    });
    let revisions = batch.revisions.iter().map(|revision| Revision {
        head: revision.head.clone(),
        root: revision.root.clone(),
        name: revision.name.clone(),
        email: revision.email.clone(),
        revision: revision.revision,
        parent: revision.parent,
        message: revision.message.clone(),
        at: revision.at,
        git_commit: revision.git_commit.clone(),
        git_meta: revision.git_meta.as_deref().map(coded),
        principal: revision.principal.clone(),
        chain: revision.chain.clone(),
    });
    let epochs = batch.epochs.iter().map(|epoch| Epoch {
        epoch: epoch.epoch,
        from_revision: epoch.from_revision,
        reason: epoch.reason.clone(),
        at: epoch.at,
    });
    let meta = batch.meta.as_ref().map(|meta| Meta {
        board_id: meta.board_id.clone(),
        algorithm: meta.algorithm.clone(),
        source_remote: meta.source_remote.clone(),
        source_ref: meta.source_ref.clone(),
        source_head: meta.source_head.clone(),
        imported_count: meta.imported_count,
    });
    let tokens = batch.tokens.iter().map(|token| Token {
        name: token.name.as_str().to_string(),
        created: token.created.as_str().to_string(),
        kind: token.kind.as_str().to_string(),
        sha256: token.digest.clone(),
    });
    let wire = Batch {
        after: batch.after,
        chain_after: batch.chain_after.clone(),
        objects: objects.collect(),
        revisions: revisions.collect(),
        epochs: epochs.collect(),
        meta,
        tokens: tokens.collect(),
        tip: batch.tip,
        more: batch.more,
    };
    serde_json::to_vec(&wire).map_err(|error| error.to_string())
}

/// The wire's JSON as a batch.
///
/// # Errors
/// What in it is not a batch.
pub fn decode_batch(bytes: &[u8]) -> Result<ReplicaBatch, String> {
    let wire: Batch = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let objects = wire.objects.into_iter().map(|object| {
        let content = match (object.blob, object.tree) {
            (Some(blob), None) => ReplicaContent::Blob(decoded(&blob)?),
            (None, Some((size, entries))) => ReplicaContent::Tree {
                size,
                entries: entries
                    .into_iter()
                    .map(|(mode, name, oid)| {
                        Ok(ReplicaEntry {
                            mode,
                            raw_name: decoded(&name)?,
                            oid,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            (_, _) => {
                return Err(format!(
                    "object {} is neither a blob nor a tree",
                    object.oid
                ));
            }
        };
        let original = match object.original {
            None => None,
            Some(Original { raw }) => Some(raw.as_deref().map(decoded).transpose()?),
        };
        Ok(ReplicaObject {
            sequence: object.sequence,
            oid: object.oid,
            content,
            original,
        })
    });
    let revisions = wire.revisions.into_iter().map(|revision| {
        Ok(ReplicaRevision {
            git_meta: revision.git_meta.as_deref().map(decoded).transpose()?,
            head: revision.head,
            root: revision.root,
            name: revision.name,
            email: revision.email,
            revision: revision.revision,
            parent: revision.parent,
            message: revision.message,
            at: revision.at,
            git_commit: revision.git_commit,
            principal: revision.principal,
            chain: revision.chain,
        })
    });
    let tokens = wire.tokens.into_iter().map(|token| {
        let unshaped = |_| format!("a token row is not as a token is kept: {}", token.name);
        let kind = match token.kind.as_str() {
            "device" => TokenKind::Device,
            "replica" => TokenKind::Replica,
            other => {
                return Err(format!(
                    "a token row's kind is not one this build reads: {other}"
                ));
            }
        };
        Ok(AdminToken {
            name: Slug::parse(&token.name).map_err(unshaped)?,
            created: Timestamp::parse(&token.created).map_err(unshaped)?,
            kind,
            digest: token.sha256,
        })
    });
    Ok(ReplicaBatch {
        after: wire.after,
        chain_after: wire.chain_after,
        objects: objects.collect::<Result<_, String>>()?,
        revisions: revisions.collect::<Result<_, String>>()?,
        epochs: wire
            .epochs
            .into_iter()
            .map(|epoch| ReplicaEpoch {
                epoch: epoch.epoch,
                from_revision: epoch.from_revision,
                reason: epoch.reason,
                at: epoch.at,
            })
            .collect(),
        meta: wire.meta.map(|meta| ReplicaMeta {
            board_id: meta.board_id,
            algorithm: meta.algorithm,
            source_remote: meta.source_remote,
            source_ref: meta.source_ref,
            source_head: meta.source_head,
            imported_count: meta.imported_count,
        }),
        tokens: tokens.collect::<Result<_, String>>()?,
        tip: wire.tip,
        more: wire.more,
    })
}

#[cfg(test)]
#[path = "board_replica_tests.rs"]
mod tests;
