use super::*;
use crate::application::project_board::ports::ArtifactKey;

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
    // A partial clone fetches every file a read may read, and none past.
    let largest = table.iter().map(|row| row.3).max().unwrap();
    assert_eq!(FILTER_BYTES, largest.max(MAX_TASK_FILE_BYTES) + 1);
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

/// A board whose `plans/` was read as a tree of `entries` entries and
/// `bytes` bytes a read reads, each entry a file a write carries.
fn carried(entries: usize, bytes: usize) -> Board {
    let files = (0..entries)
        .map(|n| {
            (
                format!("x{n}.md"),
                format!("100644 blob {}", "a".repeat(40)),
            )
        })
        .collect();
    let tree = CarriedTree {
        oid: "b".repeat(40),
        entries,
        bytes,
        files,
        ..CarriedTree::default()
    };
    Board {
        folders: BTreeMap::from([("plans", tree)]),
        ..Board::default()
    }
}

/// `plans/a.1.md`, of `size` bytes, as a write makes it.
fn plan_file(size: usize) -> Vec<FolderFile> {
    vec![FolderFile {
        folder: "plans",
        name: "a.1.md".into(),
        bytes: vec![b'x'; size],
    }]
}

/// A board's other folders, as a write leaves them, count towards what a
/// read takes in all: entries, then file bytes.
#[test]
fn a_write_counts_the_folders_it_leaves() {
    let task = a_task();
    let written = [(&task, vec![0; 10])];
    let within = |board: Board| board.within_bounds(&written, &board.folders_after(&[]), LIMITS);
    assert!(within(carried(4_096, 0)).is_ok());
    let mut full = carried(4_096, 0);
    for (name, extra) in [("runs", 4_096), ("changes", 4_096), ("reviews", 4_095)] {
        let mut tree = full.folders["plans"].clone();
        tree.entries = extra;
        tree.files = (0..extra)
            .map(|n| (format!("x{n}.md"), String::new()))
            .collect();
        full.folders.insert(name, tree);
    }
    assert!(within(full.clone()).is_ok(), "16,383 and one task");
    full.folders.get_mut("reviews").unwrap().entries = 4_096;
    full.folders
        .get_mut("reviews")
        .unwrap()
        .files
        .insert("y.md".into(), String::new());
    let past = within(full);
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("16384 entries in all")),
        "{past:?}"
    );
    let folder = |bytes| {
        let mut board = carried(1, 0);
        board.folders.get_mut("plans").unwrap().bytes = bytes;
        board
    };
    // A folder's own bytes bound it first: the board's in all is checked
    // with a limit below it.
    let limits = Limits {
        board_bytes: 100,
        ..LIMITS
    };
    let board = folder(90);
    assert!(
        board
            .within_bounds(&written, &board.folders_after(&[]), limits)
            .is_ok()
    );
    let board = folder(91);
    let past = board.within_bounds(&written, &board.folders_after(&[]), limits);
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes of files")),
        "{past:?}"
    );
}

/// A write keeps each other folder, as it leaves it, within the entries
/// and file bytes a read takes of it.
#[test]
fn a_write_keeps_each_folder_within_its_own_bounds() {
    let within = |board: &Board, files: &[FolderFile]| {
        board.within_bounds(&[], &board.folders_after(files), LIMITS)
    };
    let files = plan_file(10);
    assert!(within(&carried(4_095, 0), &files).is_ok());
    let full = within(&carried(4_096, 0), &files);
    assert!(
        matches!(&full, Err(BoardStoreError::InvalidWrite(why)) if why.contains("its plans/ holds at most 4096 plan files") && why.contains("would hold 4097")),
        "{full:?}"
    );
    let most = 24 * 1024 * 1024;
    let mut board = carried(1, 0);
    let tree = board.folders.get_mut("plans").unwrap();
    tree.sizes.insert("x0.md".into(), most - 10);
    assert!(within(&board, &files).is_ok());
    let past = within(&board, &plan_file(11));
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains(&format!("{} bytes of plan files", most + 1))),
        "{past:?}"
    );
}

/// A folder a write leaves as it was is carried as its very tree; one it
/// writes in, or one holding an entry a write drops, is rebuilt from what
/// it carries, the file written replacing the one of its name; a folder
/// never read is made by the write that writes in it.
#[test]
fn a_folder_is_carried_unless_written_in_or_dropping() {
    let board = carried(2, 0);
    let after = board.folders_after(&[]);
    assert!(matches!(after["plans"], FolderAfter::Carried(tree) if tree.oid == "b".repeat(40)));
    let mut dropping = board.clone();
    dropping.folders.get_mut("plans").unwrap().entries = 3;
    let after = dropping.folders_after(&[]);
    assert!(
        matches!(&after["plans"], FolderAfter::Rebuilt { carried, written, .. } if carried.len() == 2 && written.is_empty()),
        "{after:?}"
    );
    let mut files = plan_file(5);
    files[0].name = "x1.md".into();
    let after = board.folders_after(&files);
    assert_eq!(after["plans"].entries(), 2, "x1.md replaced, not added");
    let empty = Board::default();
    let after = empty.folders_after(&files);
    assert_eq!(
        (
            after.len(),
            after["plans"].entries(),
            after["plans"].bytes()
        ),
        (1, 1, 5)
    );
    assert!(empty.folders_after(&[]).is_empty());
}

/// A key's file lies where a read finds it: its kind's folder, under the
/// name that spells it; a task's lies in `tasks/`.
#[test]
fn a_key_is_written_where_a_read_finds_it() {
    let id = || Slug::parse("a").unwrap();
    for (key, folder, name) in [
        (ArtifactKey::Plan(id(), 1), "plans", "a.1.md"),
        (ArtifactKey::Run(id(), 999), "runs", "a.999.md"),
        (ArtifactKey::Change(id(), 2), "changes", "a.2.md"),
        (ArtifactKey::Review(id(), 3), "reviews", "a.3.md"),
        (ArtifactKey::Discussion(id()), "discussions", "a.md"),
    ] {
        let file = folder_file(&key, vec![]).unwrap();
        assert_eq!((file.folder, file.name.as_str()), (folder, name), "{key}");
    }
    assert!(folder_file(&ArtifactKey::Task(id()), vec![]).is_err());
    for key in [ArtifactKey::Plan(id(), 0), ArtifactKey::Run(id(), 1_000)] {
        let refused = folder_file(&key, vec![]);
        assert!(
            refused
                .as_ref()
                .is_err_and(|why| *why == format!("{key}: has no file name the board can read")),
            "{key}: {refused:?}"
        );
    }
    let big = folder_file(&ArtifactKey::Change(id(), 1), vec![0; 8 * 1024 + 1]);
    assert!(big.is_err_and(|why| why.contains("8193 bytes")));
}

/// A name listed twice outside `tasks/` holds only its own folder: a write
/// that writes nothing there carries the folder as its very tree, however
/// many entries it drops, and a write into it is refused, naming the clear.
#[test]
fn a_name_listed_twice_outside_tasks_holds_only_its_folder() {
    let mut board = carried(1, 0);
    assert_eq!(board.folder_repeated_refusal("c", &plan_file(1)), None);
    let copy = |oid: char| TreeCopy {
        mode: REGULAR_FILE.into(),
        kind: "blob".into(),
        oid: oid.to_string().repeat(40),
        size: 1,
    };
    let tree = board.folders.get_mut("plans").unwrap();
    tree.entries = 3;
    tree.repeated = vec![("a.1.md".into(), vec![copy('d'), copy('e')])];
    let after = board.folders_after(&[]);
    assert!(matches!(after["plans"], FolderAfter::Carried(tree) if tree.oid == "b".repeat(40)));
    let mut run = plan_file(1);
    run[0].folder = "runs";
    assert_eq!(board.folder_repeated_refusal("c", &run), None);
    let refusal = board.folder_repeated_refusal("c", &plan_file(1));
    assert!(
        refusal
            .as_deref()
            .is_some_and(|why| why.starts_with(&format!(
                "plans/a.1.md is named more than once in board commit c (objects {}, {})",
                "d".repeat(40),
                "e".repeat(40)
            )) && why.contains("no write in plans/ can carry it forward")
                && why.contains("git rejects any new tree holding it")),
        "{refusal:?}"
    );
}

/// A write keeps `tasks/` within its own bytes as well as the board within
/// its bytes in all.
#[test]
fn a_write_keeps_tasks_within_their_own_bytes() {
    let task = a_task();
    let written = [(&task, vec![0; 10])];
    let limits = |tasks_bytes| Limits {
        tasks_bytes,
        ..LIMITS
    };
    let none = BTreeMap::new();
    assert!(
        Board::default()
            .within_bounds(&written, &none, limits(10))
            .is_ok()
    );
    let past = Board::default().within_bounds(&written, &none, limits(9));
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("10 bytes of task files")),
        "{past:?}"
    );
}

/// A path set aside always shows as one name in its folder, whatever the
/// name's bytes: never empty, `.`, `..` or two names.
#[test]
fn an_entry_path_is_one_name_in_its_folder() {
    let oid = "0123456789abcdef0123456789abcdef01234567";
    let odd = "plans/\u{2026}0123456789ab";
    for (raw, shown) in [
        (&b"a.md"[..], "plans/a.md"),
        (b"\x01", odd),
        (b"\x01.\x01", odd),
        (b"\x01..", odd),
        (b".", odd),
        (b"a/b", odd),
        (b"a\x01b", "plans/a b"),
        (b"a..b", "plans/a..b"),
        ("\u{2026}".as_bytes(), "plans/\u{2026}"),
    ] {
        let path = entry_path("plans", raw, oid);
        assert_eq!(path, shown, "{raw:?}");
        assert!(
            crate::domain::project_board::value_objects::shown_text::shown_board_path(&path),
            "{path:?}"
        );
    }
}
