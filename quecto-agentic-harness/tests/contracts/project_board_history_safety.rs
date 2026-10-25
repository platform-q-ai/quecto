//! Scoped A sink safety; original import safety deferred intact.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::history::*;
use std::path::Path;
fn assert_projection(entries: &[(&str, &[u8], &str)]) {
    let allocation: usize = entries
        .iter()
        .map(|(_, name, oid)| 128 + 6 + oid.len() + 4 * name.len())
        .sum();
    assert!(
        allocation <= 4 * 655360,
        "positive boundary fixture must satisfy independent normalized listing allocation guard: {allocation}"
    );
    let logical: usize = entries
        .iter()
        .map(|(mode, name, oid)| {
            mode.trim_start_matches('0').len() + 1 + name.len() + 1 + oid.len() / 2
        })
        .sum();
    assert!(
        logical <= 655360,
        "positive fixture independently fits logical tree admission"
    );
}

fn checked_tree(repo: &Path, entries: &[(&str, &[u8], &str)]) -> String {
    assert_projection(entries);
    raw_tree(repo, entries)
}

#[test]
fn feature_red_sink_native_tree_uses_git_directory_sort_and_binary_oid_width() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let blob = hash(&fixture.destination, "blob", b"native bytes\0");
    let directory = checked_tree(&fixture.destination, &[("100644", b"child", &blob)]);
    let entries = vec![
        HistoryEntry {
            mode: "040000".into(),
            name: b"a".to_vec(),
            oid: directory.clone(),
        },
        HistoryEntry {
            mode: "100644".into(),
            name: b"a.c".to_vec(),
            oid: blob.clone(),
        },
    ];
    let listing = format!("040000 tree {directory}\ta\n100644 blob {blob}\ta.c\n");
    let expected = String::from_utf8(git(&fixture.destination, &["mktree"], listing.as_bytes()))
        .unwrap()
        .trim()
        .to_string();
    let actual = fixture
        .handles()
        .sink
        .tree(&entries, true)
        .expect("pure real Git tree codec");
    assert_eq!(actual, expected);
    let bytes = git(&fixture.destination, &["cat-file", "tree", &expected], b"");
    assert!(
        bytes.starts_with(b"100644 a.c\0"),
        "directory-aware order differs from lexical a-before-a.c"
    );
    assert_eq!(bytes.len(), 6 + 1 + 3 + 1 + 20 + 5 + 1 + 1 + 1 + 20);
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
}

#[test]
fn feature_red_sink_wrong_original_oid_is_malformed_not_published() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let result = fixture.handles().sink.raw(
        HistoryKind::Blob,
        b"correct body",
        Some(&"0".repeat(40)),
        false,
    );
    assert!(
        matches!(result, Err(HistoryError::Malformed(_))),
        "expected raw OID must be verified against full kind+bytes before any ref publication: {result:?}"
    );
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
}
