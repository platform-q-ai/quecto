use super::*;
use crate::domain::project_board::value_objects::telemetry::BoardId;

fn moved() -> BoardMoved {
    let id = BoardId::of_hex("0123456789abcdef").unwrap();
    BoardMoved::new(id, vec!["https://board.example:7443".into()]).unwrap()
}

fn entry(meta: &str) -> String {
    meta.to_string()
}

/// The fence's `format` tree holds the format-4 marker and MOVED, alone.
#[test]
fn a_fences_format_tree_holds_the_format_4_marker_and_moved() {
    let files = fence_files(&moved());
    assert_eq!(
        files,
        BTreeMap::from([
            (FORMAT_FILE, b"quecto-board 4\n".to_vec()),
            (MOVED_FILE, moved().file()),
        ])
    );
    assert_eq!(MOVED_FORMAT, b"quecto-board 4\n");
}

/// The fence's root keeps the README, `tasks`, the other folders and the
/// archive as they are, replaces the format tree, and drops strays.
#[test]
fn a_fences_root_replaces_the_format_tree_and_drops_strays() {
    let tree = |oid: char| format!("040000 tree {}", oid.to_string().repeat(40));
    let root = BTreeMap::from([
        (
            "README.md".to_string(),
            entry(&format!("100644 blob {}", "1".repeat(40))),
        ),
        ("tasks".to_string(), tree('2')),
        ("plans".to_string(), tree('3')),
        ("archive".to_string(), tree('4')),
        (FORMAT_TREE.to_string(), tree('5')),
        (
            "notes.txt".to_string(),
            entry(&format!("100644 blob {}", "6".repeat(40))),
        ),
    ]);
    let format = "7".repeat(40);
    let fenced = fenced_root(&root, &format);
    let mut expected = root.clone();
    expected.remove("notes.txt");
    expected.insert(FORMAT_TREE.to_string(), format!("040000 tree {format}"));
    assert_eq!(fenced, expected);
    // A board written before the marker gains one.
    let mut legacy = root.clone();
    legacy.remove(FORMAT_TREE);
    assert_eq!(fenced_root(&legacy, &format), expected);
    // Fencing again with the same tree changes nothing.
    assert_eq!(fenced_root(&fenced, &format), fenced);
}
