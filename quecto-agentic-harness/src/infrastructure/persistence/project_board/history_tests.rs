use super::*;

#[test]
fn source_metadata_32m_is_inclusive_and_checked_overflow_refuses() {
    let mut metadata = META - 512;
    charge(&mut metadata, 512, META).unwrap();
    assert_eq!(metadata, META);
    assert!(matches!(
        charge(&mut metadata, 1, META),
        Err(HistoryError::Bounds(_))
    ));
    let mut overflow = usize::MAX;
    assert!(matches!(
        charge(&mut overflow, 1, usize::MAX),
        Err(HistoryError::Bounds(_))
    ));
    assert_eq!(overflow, usize::MAX);
}

fn tree(id: String, entries: Vec<HistoryEntry>) -> HistoryObject {
    HistoryObject {
        oid: id,
        kind: HistoryKind::Tree,
        bytes: vec![],
        entries,
    }
}
#[test]
fn source_depth128_root_zero_is_inclusive_and_reused_deeper_tree_refuses() {
    let mut objects = BTreeMap::new();
    let mut previous = "0".repeat(40);
    objects.insert(previous.clone(), tree(previous.clone(), vec![]));
    for n in 1..=128 {
        let id = format!("{n:040x}");
        objects.insert(
            id.clone(),
            tree(
                id.clone(),
                vec![HistoryEntry {
                    mode: "040000".into(),
                    name: b"next".to_vec(),
                    oid: previous,
                }],
            ),
        );
        previous = id;
    }
    let mut heights = BTreeMap::new();
    assert_eq!(
        height(&previous, &objects, &mut heights, &mut BTreeSet::new(), 0).unwrap(),
        128
    );
    assert!(matches!(
        height(&previous, &objects, &mut heights, &mut BTreeSet::new(), 1),
        Err(HistoryError::Bounds(_))
    ));
}
#[test]
fn source_hashes_full_actual_kind_framing_both_formats() {
    for (width, id) in [
        (40, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
        (
            64,
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813",
        ),
    ] {
        let mut object = HistoryObject {
            oid: id.into(),
            kind: HistoryKind::Blob,
            bytes: vec![],
            entries: vec![],
        };
        verify_raw(&object, width).unwrap();
        object.kind = HistoryKind::Tree;
        assert!(matches!(
            verify_raw(&object, width),
            Err(HistoryError::Malformed(_))
        ));
        object.kind = HistoryKind::Blob;
        object.bytes.push(0);
        assert!(matches!(
            verify_raw(&object, width),
            Err(HistoryError::Malformed(_))
        ));
    }
}
#[test]
fn source_projection_preallocation_checks_frames_and_name_bounds() {
    let raw = |n| {
        let mut b = b"100644 ".to_vec();
        b.extend(vec![b'x'; n]);
        b.push(0);
        b.extend([0; 20]);
        b
    };
    assert_eq!(
        projection_budget(&raw(4096), 40).unwrap(),
        (1, 128 + 6 + 40 + 4 * 4096)
    );
    assert!(matches!(
        projection_budget(&raw(4097), 40),
        Err(HistoryError::Bounds(_))
    ));
    let mut truncated = raw(1);
    truncated.pop();
    assert!(matches!(
        projection_budget(&truncated, 40),
        Err(HistoryError::Malformed(_))
    ));
}

#[test]
fn source_required_identity_headers_refuse_empty_name_and_email() {
    for identity in [" <h@example.com>", "History <>"] {
        let bytes = format!(
            "tree {}\nauthor {identity} 1 +0000\ncommitter History <h@example.com> 1 +0000\n\nbody",
            "0".repeat(40)
        );
        assert!(matches!(
            commit_links(bytes.as_bytes(), 40),
            Err(HistoryError::Malformed(_))
        ));
    }
}
