//! A batch on the wire (epic #2705 B11): whole both ways, read strictly.
use super::*;

fn batch() -> ReplicaBatch {
    ReplicaBatch {
        after: 2,
        chain_after: Some("c2".into()),
        objects: vec![
            ReplicaObject {
                sequence: 7,
                oid: "aa".into(),
                content: ReplicaContent::Blob(vec![0, 255, 10]),
                original: None,
            },
            ReplicaObject {
                sequence: 8,
                oid: "bb".into(),
                content: ReplicaContent::Tree {
                    size: 40,
                    entries: vec![ReplicaEntry {
                        mode: "100644".into(),
                        raw_name: b"a\xffb".to_vec(),
                        oid: "aa".into(),
                    }],
                },
                original: Some(Some(b"raw tree".to_vec())),
            },
            ReplicaObject {
                sequence: 9,
                oid: "cc".into(),
                content: ReplicaContent::Blob(vec![]),
                original: Some(None),
            },
        ],
        revisions: vec![ReplicaRevision {
            head: "1:3".into(),
            root: "bb".into(),
            name: "Ada".into(),
            email: "ada@example.com".into(),
            revision: 3,
            parent: Some(2),
            message: "board: edit".into(),
            at: 1_700_000_000,
            git_commit: Some("dd".into()),
            git_meta: Some(b"tree bb\n".to_vec()),
            principal: Some("token:phone".into()),
            chain: "c3".into(),
        }],
        epochs: vec![ReplicaEpoch {
            epoch: 1,
            from_revision: 1,
            reason: "bootstrap".into(),
            at: 5,
        }],
        meta: Some(ReplicaMeta {
            board_id: Some("0123456789abcdef".into()),
            algorithm: "sha256".into(),
            source_remote: None,
            source_ref: None,
            source_head: None,
            imported_count: 0,
        }),
        tokens: vec![AdminToken {
            name: Slug::parse("phone").unwrap(),
            created: Timestamp::parse("2026-10-26T09:00:00Z").unwrap(),
            digest: "ab".repeat(32),
        }],
        tip: 9,
        more: true,
    }
}

#[test]
fn a_batch_crosses_the_wire_whole() {
    let sent = batch();
    let bytes = encode_batch(&sent).unwrap();
    assert_eq!(decode_batch(&bytes).unwrap(), sent);
}

#[test]
fn a_batch_is_read_strictly() {
    let bytes = encode_batch(&batch()).unwrap();
    let mut wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    wire["extra"] = serde_json::json!(1);
    assert!(decode_batch(wire.to_string().as_bytes()).is_err());
    let mut both: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    both["objects"][0]["tree"] = serde_json::json!([1, []]);
    let refused = decode_batch(both.to_string().as_bytes()).unwrap_err();
    assert!(refused.contains("neither a blob nor a tree"), "{refused}");
    let mut token: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    token["tokens"][0]["name"] = serde_json::json!("Not A Slug");
    assert!(decode_batch(token.to_string().as_bytes()).is_err());
}
