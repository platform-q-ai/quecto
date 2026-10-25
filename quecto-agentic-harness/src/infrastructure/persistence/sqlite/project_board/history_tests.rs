use super::*;
use sha2::Digest;
fn original(kind: HistoryKind, bytes: Vec<u8>, algorithm: HistoryAlgorithm) -> HistoryObject {
    let name = match kind {
        HistoryKind::Tree => "tree",
        HistoryKind::Commit => "commit",
        HistoryKind::Blob => "blob",
    };
    let frame = format!("{name} {}\0", bytes.len());
    let oid = match algorithm {
        HistoryAlgorithm::Sha1 => {
            let mut h = sha1::Sha1::new();
            h.update(frame.as_bytes());
            h.update(&bytes);
            format!("{:x}", h.finalize())
        }
        HistoryAlgorithm::Sha256 => {
            let mut h = sha2::Sha256::new();
            h.update(frame.as_bytes());
            h.update(&bytes);
            format!("{:x}", h.finalize())
        }
    };
    HistoryObject {
        oid,
        kind,
        bytes,
        entries: vec![],
    }
}
fn fixture(path: PathBuf, algorithm: HistoryAlgorithm) -> SqliteBoardHistoryStore {
    let writer = Identity {
        name: "History".into(),
        email: "h@example.com".into(),
    };
    let store = SqliteBoardHistoryStore::new(
        path,
        writer.clone(),
        BoardId::of_hex("0123456789abcdef").unwrap(),
    );
    let tasks = original(HistoryKind::Tree, vec![], algorithm);
    let mut raw = b"40000 tasks\0".to_vec();
    for pair in tasks.oid.as_bytes().chunks_exact(2) {
        raw.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
    }
    let mut tree = original(HistoryKind::Tree, raw, algorithm);
    tree.entries.push(HistoryEntry {
        mode: "040000".into(),
        name: b"tasks".to_vec(),
        oid: tasks.oid.clone(),
    });
    let commit=original(HistoryKind::Commit,format!("tree {}\nauthor History <h@example.com> 1 +0000\ncommitter History <h@example.com> 1 +0000\n\noriginal\n",tree.oid).into_bytes(),algorithm);
    let h = LinearHistory {
        algorithm,
        source_ref: "refs/heads/quecto/board".into(),
        source_remote: "local".into(),
        source_head: commit.oid.clone(),
        objects: vec![tasks, tree.clone()],
        revisions: vec![HistoryRevision {
            sequence: 1,
            parent: None,
            root: tree.oid,
            writer,
            message: "original".into(),
            at: 1,
            original: Some(commit),
        }],
    };
    store.prepare_import(&h).unwrap().commit().unwrap();
    store
}
#[test]
fn imported_native_unchanged_clear_can_carry_original_root_both_formats() {
    for algorithm in [HistoryAlgorithm::Sha1, HistoryAlgorithm::Sha256] {
        let temp = tempfile::tempdir().unwrap();
        let store = fixture(temp.path().join("board.db"), algorithm);
        let root = store
            .snapshot()
            .unwrap()
            .next_revision(0)
            .unwrap()
            .unwrap()
            .root;
        let c = storage::open(&store.path).unwrap();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let mut writer = storage::ObjectWriter::new(&c);
        writer.insert("INSERT INTO revisions VALUES ('2:2',?1,'History','h@example.com',2,1,'Clear',2,NULL,NULL)",[&root]).unwrap();
        writer
            .insert("INSERT INTO epochs VALUES (2,2,'clear',2)", [])
            .unwrap();
        writer
            .insert("UPDATE state SET head='2:2' WHERE id=1", [])
            .unwrap();
        c.execute_batch("COMMIT").unwrap();
        let view = store.snapshot().unwrap();
        assert_eq!(view.algorithm().unwrap(), Some(algorithm));
        let native = view.next_revision(1).unwrap().unwrap();
        assert_eq!(native.root, root);
        assert_eq!(native.parent, Some(1));
        assert_eq!(native.original, None);
        assert!(view.is_original(&native.root).unwrap());
        assert_eq!(view.object(&native.root).unwrap().kind, HistoryKind::Tree);
    }
}
#[test]
fn imported_root_membership_must_be_original_actual_tree_before_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path().join("board.db"), HistoryAlgorithm::Sha1);
    let c = storage::open(&store.path).unwrap();
    c.execute("UPDATE git_objects SET raw_tree=NULL", [])
        .unwrap();
    c.execute(
        "UPDATE objects SET kind='blob',bytes=x'',tree_size=NULL",
        [],
    )
    .unwrap();
    assert!(matches!(store.snapshot(), Err(HistoryError::Malformed(_))));
}

#[test]
fn import_dto_projection_messages_are_charged_to_metadata_before_rows() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path().join("board.db"), HistoryAlgorithm::Sha256);
    let revision = store.snapshot().unwrap().next_revision(0).unwrap().unwrap();
    let mut h = LinearHistory {
        algorithm: HistoryAlgorithm::Sha256,
        source_ref: "refs/heads/quecto/board".into(),
        source_remote: "local".into(),
        source_head: revision.original.as_ref().unwrap().oid.clone(),
        objects: vec![],
        revisions: vec![revision; 513],
    };
    for r in &mut h.revisions {
        r.message = "x".repeat(65536);
    }
    assert!(matches!(admission(&h), Err(HistoryError::Bounds(_))));
}

#[test]
fn imported_h0_id_type_and_width_refuse_before_rust_decode() {
    use rusqlite::types::Value;
    for bad in [Value::Text("0".repeat(65537)), Value::Blob(vec![b'0'; 40])] {
        let temp = tempfile::tempdir().unwrap();
        let store = fixture(temp.path().join("board.db"), HistoryAlgorithm::Sha1);
        storage::open(&store.path)
            .unwrap()
            .execute("UPDATE revisions SET git_commit=?1 WHERE revision=1", [bad])
            .unwrap();
        assert!(matches!(store.snapshot(), Err(HistoryError::Malformed(_))));
    }
}
