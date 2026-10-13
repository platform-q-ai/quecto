//! The bounds a read of the board's folders takes (#2481 S9): each
//! folder's entries and file bytes, `tasks/` with the rest, and the board's
//! own in all, every one judged before any file's bytes are read.
use super::*;

/// `count` rows of one blob in `repo`, named by `name`.
fn many(repo: &Path, count: usize, bytes: &[u8], name: impl Fn(usize) -> String) -> Vec<Row> {
    let oid = blob_in(repo, bytes);
    (1..=count)
        .map(|n| ("100644", name(n), oid.clone()))
        .collect()
}

/// Each folder is read within its own bounds (entries, then bytes), and
/// the board within its own: past any, it is not read.
#[test]
fn a_board_past_a_folders_bounds_or_its_own_is_not_read() {
    let refused = |rows: &dyn Fn(&Path) -> Vec<RootRow>, bound: &str| {
        let repos = fresh();
        board_of(&repos, |repo| {
            let mut rows = rows(repo);
            rows.push(("40000", "tasks", Node::Tree(vec![])));
            rows
        });
        let read = store(&repos.bob).read();
        assert!(
            matches!(&read, Err(BoardStoreError::TooLarge(why)) if why.contains(bound)),
            "{bound}: {read:?}"
        );
    };
    refused(
        &|repo| {
            let rows = many(repo, 4_097, b"x", |n| format!("x{n}.txt"));
            vec![("40000", "changes", Node::Made(rows))]
        },
        "changes tree holds 4097 entries",
    );
    refused(
        &|repo| {
            let rows = many(repo, 513, &[b'x'; 8 * 1024], |n| format!("a.{n}.md"));
            vec![("40000", "changes", Node::Made(rows))]
        },
        "changes files hold 4202496 bytes",
    );
    // The folders' 16,384 entries and one in `tasks/`: the board's own
    // bound counts every folder, `tasks/` with the rest.
    let all = |repo: &Path, tasks: usize| -> Vec<RootRow> {
        let mut rows: Vec<RootRow> = ["plans", "runs", "changes", "reviews"]
            .into_iter()
            .map(|name| {
                let rows = many(repo, 4_096, b"x", |n| format!("x{n}.txt"));
                ("40000", name, Node::Made(rows))
            })
            .collect();
        let strays = many(repo, tasks, b"x", |n| format!("x{n}.txt"));
        rows.push(("40000", "tasks", Node::Made(strays)));
        rows
    };
    let repos = fresh();
    board_of(&repos, |repo| all(repo, 1));
    let read = store(&repos.bob).read();
    assert!(
        matches!(&read, Err(BoardStoreError::TooLarge(why)) if why.contains("16385 entries")),
        "{read:?}"
    );
    let repos = fresh();
    board_of(&repos, |repo| all(repo, 0));
    assert!(store(&repos.bob).read().is_ok(), "16,384 entries are read");
}

/// The board's file bytes in all count every folder, `tasks/` with the
/// rest: at the bound it is read, past it it is not.
#[test]
fn a_board_past_its_bytes_in_all_is_not_read() {
    let repos = fresh();
    board_of(&repos, |_| every_kind());
    let all = encode(&task("a")).unwrap().len()
        + bytes::<PlanCodec>(&plan("a", 1)).len()
        + bytes::<RunCodec>(&run("a", 1)).len()
        + bytes::<ChangeCodec>(&change("a", 1)).len()
        + bytes::<ReviewCodec>(&review("a", 1)).len()
        + bytes::<DiscussionCodec>(&discussion("d")).len();
    let mut bob = store(&repos.bob);
    bob.limits.board_bytes = all;
    assert!(bob.read().is_ok(), "at the bound");
    let mut bob = store(&repos.bob);
    bob.limits.board_bytes = all - 1;
    let read = bob.read();
    assert!(
        matches!(&read, Err(BoardStoreError::TooLarge(why)) if why.contains(&format!("files hold {all} bytes"))),
        "{read:?}"
    );
}

/// `tasks/` is read within its own bytes (16 MiB), as each folder is, and
/// a write keeps within them.
#[test]
fn tasks_are_read_and_written_within_their_own_bytes() {
    assert_eq!(LIMITS.tasks_bytes, 16 * 1024 * 1024);
    let repos = fresh();
    board_of(&repos, |_| every_kind());
    let size = encode(&task("a")).unwrap().len();
    let mut bob = store(&repos.bob);
    bob.limits.tasks_bytes = size - 1;
    let read = bob.read();
    assert!(
        matches!(&read, Err(BoardStoreError::TooLarge(why)) if why.contains("task files hold")),
        "{read:?}"
    );
    let mut bob = store(&repos.bob);
    bob.limits.tasks_bytes = size;
    let head = bob.read().unwrap().head;
    let write = bob.write_tasks(&head, &[task("b")], "board: add b");
    assert!(
        matches!(&write, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes of task files")),
        "{write:?}"
    );
}

/// A git on the store's `PATH` that logs every object a `cat-file --batch`
/// is asked to read (what a read reads the bytes of), then runs git.
fn logging_git(dir: &Path) -> (std::ffi::OsString, PathBuf) {
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|git| git.is_file())
        .unwrap();
    let bin = dir.join("logging-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.join("batch.log");
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in\n  *\" --batch \"*) tee -a '{log}' | '{real}' \"$@\" ;;\n  \
         *) exec '{real}' \"$@\" ;;\nesac\n",
        log = log.display(),
        real = real.display()
    );
    executable(&bin.join("git"), &script);
    let mut path = std::ffi::OsString::from(bin.as_os_str());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    (path, log)
}

/// A board past what a read takes, in a folder or in all, is refused
/// before any file's bytes are read: only the format marker's are.
#[test]
fn a_board_past_its_bounds_is_refused_before_any_file_is_read() {
    let refused_reading_only_the_marker =
        |rows: &dyn Fn(&Path) -> Vec<RootRow>, limits: Limits, bound: &str| {
            let repos = fresh();
            let head = board_of(&repos, rows);
            let (path, log) = logging_git(repos.dir.path());
            let mut vars = vars(&repos.bob, &[]);
            vars.retain(|(key, _)| key != "PATH");
            vars.push(("PATH".into(), path));
            let mut bob = store_with(&repos.bob, vars);
            bob.limits = limits;
            let read = bob.board_at(&head.0);
            assert!(
                matches!(&read, Err(BoardStoreError::TooLarge(why)) if why.contains(bound)),
                "{bound}: {read:?}"
            );
            let marker = at(&repos.bob, &head, "format/board");
            let asked = std::fs::read_to_string(&log).unwrap_or_default();
            let asked: Vec<&str> = asked.lines().collect();
            assert!(
                !asked.is_empty(),
                "{bound}: the log records the marker's read"
            );
            assert!(asked.iter().all(|oid| *oid == marker), "{bound}: {asked:?}");
        };
    // Past a folder's bytes, beside files of every kind within theirs.
    refused_reading_only_the_marker(
        &|repo| {
            let mut rows = every_kind();
            rows.remove(3);
            let big = many(repo, 513, &[b'x'; 8 * 1024], |n| format!("a.{n}.md"));
            rows.push(("40000", "changes", Node::Made(big)));
            rows
        },
        LIMITS,
        "changes files hold",
    );
    // Every folder within its bytes, the board past its own.
    let small = Limits {
        board_bytes: 1,
        ..LIMITS
    };
    refused_reading_only_the_marker(&|_| every_kind(), small, "the board's files hold");
}
