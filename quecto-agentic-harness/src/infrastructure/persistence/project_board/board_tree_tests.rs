use super::*;

fn entry(raw: &[u8]) -> Entry {
    Entry {
        mode: REGULAR_FILE.into(),
        oid: "a".repeat(40),
        name: String::from_utf8_lossy(raw).into_owned(),
        raw: raw.to_vec(),
    }
}

/// Every name quecto keeps shows exactly as it is, whatever byte it
/// holds: tried for every byte, alone and after a letter.
#[test]
fn every_name_quecto_keeps_shows_as_it_is() {
    let mut kept = 0;
    for byte in 0..=u8::MAX {
        for raw in [vec![byte], vec![b'a', byte], vec![b'a', byte, b'z']] {
            let name = entry(&raw);
            if name.kept_name(MAX_NAME_BYTES) {
                kept += 1;
                assert_eq!(shown_name(&raw).as_bytes(), raw.as_slice(), "{raw:?}");
            }
        }
    }
    assert!(kept > 62 * 3, "letters, digits and punctuation are kept");
}

/// The folders and their read bounds are the plan's (#2481 M3): entries
/// and file bytes per folder, each kind's codec cap per file, and names in
/// the order `ARTIFACT_FOLDERS` lists them.
#[test]
fn the_folders_and_their_bounds_are_the_plans() {
    use crate::domain::project_board::value_objects::shown_text::ARTIFACT_FOLDERS;
    const MIB: usize = 1024 * 1024;
    let table: Vec<(&str, usize, usize, usize)> = FOLDERS
        .iter()
        .map(|folder| {
            let naming = &folder.naming;
            (
                naming.folder,
                folder.max_entries,
                folder.max_bytes,
                naming.file_bytes,
            )
        })
        .collect();
    assert_eq!(
        table,
        [
            ("plans", 4_096, 24 * MIB, 1_536 * 1024),
            ("runs", 4_096, 16 * MIB, 768 * 1024),
            ("changes", 4_096, 4 * MIB, 8 * 1024),
            ("reviews", 4_096, 16 * MIB, 1_024 * 1024),
            ("discussions", 2_048, 16 * MIB, MIB),
        ]
    );
    let names: Vec<&str> = FOLDERS.iter().map(|folder| folder.naming.folder).collect();
    assert_eq!(names, ARTIFACT_FOLDERS);
    assert_eq!(MAX_BOARD_ENTRIES, 16_384);
    for name in ["tasks", "plans", "discussions", ARCHIVE] {
        assert!(root_tree(name), "{name}");
    }
    for name in ["format", "README.md", "src", "archives", "plan", ""] {
        assert!(!root_tree(name), "{name}");
    }
}

/// A name spells a key only as `<id>.<n>` with `n` from 1 to 999 and no
/// leading zero, or a discussion's `<id>`.
#[test]
fn a_folder_name_spells_a_key_only_as_written() {
    use crate::application::project_board::ports::ArtifactKey;
    let id = || Slug::parse("a").unwrap();
    let [plans, runs, changes, reviews, discussions] = &FOLDERS;
    assert_eq!((plans.spell)("a.1"), Some(ArtifactKey::Plan(id(), 1)));
    assert_eq!((runs.spell)("a.999"), Some(ArtifactKey::Run(id(), 999)));
    assert_eq!((changes.spell)("a.42"), Some(ArtifactKey::Change(id(), 42)));
    assert_eq!((reviews.spell)("a.7"), Some(ArtifactKey::Review(id(), 7)));
    assert_eq!(
        (discussions.spell)("a"),
        Some(ArtifactKey::Discussion(id()))
    );
    for stem in [
        "a", "a.0", "a.01", "a.1000", "a.+1", "a.1a", "a.", ".1", "A.1", "a.1.1", "a..1",
    ] {
        assert_eq!((plans.spell)(stem), None, "{stem:?}");
    }
    for stem in ["a.1", "A", "", "con"] {
        assert_eq!((discussions.spell)(stem), None, "{stem:?}");
    }
}

fn a_task() -> Task {
    use crate::domain::project_board::entities::task::TaskFields;
    use crate::domain::project_board::value_objects::timestamp::Timestamp;
    use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse("a").unwrap(),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description: "Do it.".into(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

/// A board's folders a write carries count towards what a read takes in
/// all: entries, then file bytes.
#[test]
fn a_write_counts_the_folders_it_carries() {
    let carried = |entries: usize, bytes: usize| Board {
        folders: BTreeMap::from([(
            "plans",
            CarriedTree {
                oid: "a".repeat(40),
                entries,
                bytes,
            },
        )]),
        ..Board::default()
    };
    let task = a_task();
    let written = [(&task, vec![0; 10])];
    assert!(
        carried(MAX_BOARD_ENTRIES - 1, 0)
            .within_bounds(&written, LIMITS)
            .is_ok()
    );
    let full = carried(MAX_BOARD_ENTRIES, 0).within_bounds(&written, LIMITS);
    assert!(
        matches!(&full, Err(BoardStoreError::InvalidWrite(why)) if why.contains("16384 entries in all")),
        "{full:?}"
    );
    let bytes = MAX_BOARD_BYTES - 10;
    assert!(carried(1, bytes).within_bounds(&written, LIMITS).is_ok());
    let past = carried(1, bytes + 1).within_bounds(&written, LIMITS);
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes of files")),
        "{past:?}"
    );
}

/// Only a file set aside in a folder besides `tasks/` stops a write.
#[test]
fn only_a_file_set_aside_in_another_folder_stops_a_write() {
    let aside = |path: &str| QuarantinedFile {
        path: path.into(),
        key: None,
        reason: "a stray".into(),
        object: "a".repeat(40),
        carried: false,
    };
    let board = |paths: &[&str]| Board {
        quarantined: paths.iter().map(|path| aside(path)).collect(),
        ..Board::default()
    };
    assert_eq!(
        board(&["tasks/x.txt", "tasks/plans/x"]).folders_refusal(),
        None
    );
    let refusal = board(&["discussions/x", "plans/x.txt", "tasks/x.txt"]).folders_refusal();
    assert!(
        refusal
            .as_deref()
            .is_some_and(|why| why.starts_with("discussions/x is set aside")),
        "{refusal:?}"
    );
    assert_eq!(board(&["plansx/a"]).folders_refusal(), None);
}
