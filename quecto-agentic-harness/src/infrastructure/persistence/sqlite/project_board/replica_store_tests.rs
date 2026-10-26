//! An object is as its id says (epic #2705 B11 review): a native one by
//! its content's id, an imported original by its git id in its board's
//! algorithm, a tree by the entries its raw bytes hold.
use super::*;
use sha2::Digest;

/// The git id of `kind` `bytes` in `width` (40: SHA-1, 64: SHA-256).
fn git_id(kind: &str, bytes: &[u8], width: usize) -> String {
    let head = format!("{kind} {}\0", bytes.len());
    match width {
        40 => {
            let mut hash = sha1::Sha1::new();
            hash.update(head.as_bytes());
            hash.update(bytes);
            format!("{:x}", hash.finalize())
        }
        _ => {
            let mut hash = sha2::Sha256::new();
            hash.update(head.as_bytes());
            hash.update(bytes);
            format!("{:x}", hash.finalize())
        }
    }
}

fn blob(oid: String, bytes: &[u8], original: Option<Option<Vec<u8>>>) -> ReplicaObject {
    ReplicaObject {
        sequence: 1,
        oid,
        content: ReplicaContent::Blob(bytes.to_vec()),
        original,
    }
}

/// An imported tree holding one blob, `name`, in `width`.
fn tree(width: usize, name: &[u8]) -> ReplicaObject {
    let entry = git_id("blob", b"x", width);
    let mut raw = [b"100644 ", name, b"\0"].concat();
    for pair in entry.as_bytes().chunks_exact(2) {
        raw.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
    }
    let entries = vec![ReplicaEntry {
        mode: "100644".into(),
        raw_name: name.to_vec(),
        oid: entry.clone(),
    }];
    let size = crate::infrastructure::persistence::project_board::board_tree::raw_entry_bytes(
        "100644", name, &entry,
    );
    ReplicaObject {
        sequence: 1,
        oid: git_id("tree", &raw, width),
        content: ReplicaContent::Tree {
            size: size as u64,
            entries,
        },
        original: Some(Some(raw)),
    }
}

#[test]
fn a_native_object_is_its_contents_id() {
    assert!(as_its_id_says(&blob(storage::blob_id(b"x"), b"x", None)));
    assert!(!as_its_id_says(&blob(storage::blob_id(b"x"), b"y", None)));
}

#[test]
fn an_imported_original_is_its_git_id_in_either_algorithm() {
    for width in [40, 64] {
        let id = git_id("blob", b"x", width);
        assert!(
            as_its_id_says(&blob(id.clone(), b"x", Some(None))),
            "{width}"
        );
        assert!(!as_its_id_says(&blob(id, b"y", Some(None))), "{width}");
        assert!(as_its_id_says(&tree(width, b"a.md")), "{width}");
        let mut renamed = tree(width, b"a.md");
        if let ReplicaContent::Tree { entries, .. } = &mut renamed.content {
            entries[0].raw_name = b"b.md".to_vec();
        }
        assert!(
            !as_its_id_says(&renamed),
            "entries not its raw tree's: {width}"
        );
        let mut resized = tree(width, b"a.md");
        if let ReplicaContent::Tree { size, .. } = &mut resized.content {
            *size += 1;
        }
        assert!(
            !as_its_id_says(&resized),
            "a size not its entries': {width}"
        );
    }
}

#[test]
fn an_original_of_another_shape_is_refused() {
    let id = git_id("blob", b"x", 40);
    assert!(!as_its_id_says(&blob(
        id.clone(),
        b"x",
        Some(Some(b"raw".to_vec()))
    )));
    let short = id[..39].to_string();
    assert!(
        !as_its_id_says(&blob(short, b"x", Some(None))),
        "a width no algorithm has"
    );
    let mut rawless = tree(64, b"a.md");
    rawless.original = Some(None);
    assert!(!as_its_id_says(&rawless));
}
