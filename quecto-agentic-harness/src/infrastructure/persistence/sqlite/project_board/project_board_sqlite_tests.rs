use super::*;
use crate::infrastructure::persistence::project_board::board_tree::Reader;

fn client(path: &Path) -> SqliteProjectBoardStore {
    SqliteProjectBoardStore::open(
        path,
        Identity {
            name: "Child".into(),
            email: "child@example.com".into(),
        },
    )
    .unwrap()
}

#[test]
fn uncommitted_owned_child_crash_rolls_back_rows_and_head() {
    const CHILD: &str = "QUECTO_B2_UNCOMMITTED_DATABASE";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = Path::new(&path);
        let store = client(path);
        let mut c = store.connection.lock().unwrap();
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let (_, root) = storage::current(&tx).unwrap();
        let mut roots = store
            .reader(&tx)
            .listing(
                &root,
                4 * crate::infrastructure::persistence::project_board::board_tree::MAX_TREE_BYTES,
            )
            .unwrap();
        let oid = storage::blob(&tx, b"uncommitted bytes").unwrap();
        let tasks = storage::tree(&tx, &[storage::entry("100644", b"partial.md", oid)]).unwrap();
        roots.retain(|entry| entry.name != "tasks");
        roots.push(storage::entry("040000", b"tasks", tasks));
        let root = storage::tree(&tx, &roots).unwrap();
        storage::advance(&tx, &root, &store.writer).unwrap();
        std::fs::write(path.with_extension("ready"), b"transaction open").unwrap();
        loop {
            std::thread::park();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash.sqlite");
    let store = client(&path);
    store.bootstrap().unwrap();
    let before = store.read().unwrap();
    let before_raw = store.fixture_fingerprint().unwrap();
    let output = tempfile::tempfile().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "infrastructure::persistence::sqlite::project_board::project_board_sqlite_tests::uncommitted_owned_child_crash_rolls_back_rows_and_head", "--nocapture"])
        .env(CHILD, &path).stdout(output.try_clone().unwrap()).stderr(output).spawn().unwrap();
    let started = std::time::Instant::now();
    let ready = loop {
        if path.with_extension("ready").exists() {
            break true;
        }
        if started.elapsed() < Duration::from_secs(10) && child.try_wait().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        break false;
    };
    let killed = child.kill();
    let reaped = child.wait();
    assert!(ready, "child did not open transaction within 10s");
    assert!(killed.is_ok() && reaped.is_ok(), "kill/reap owned child");
    assert_eq!(client(&path).read().unwrap(), before);
    assert_eq!(
        client(&path).fixture_fingerprint().unwrap(),
        before_raw,
        "uncommitted orphan objects/tree rows/revisions/state also roll back"
    );
    assert!(rows(&store.connection.lock().unwrap()).unwrap().is_empty());
    let c = store.connection.lock().unwrap();
    let revisions: i64 = c
        .query_row("SELECT count(*) FROM revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(revisions, 1, "uncommitted provenance also rolled back");
}

fn rows(c: &Connection) -> Result<Vec<String>, BoardStoreError> {
    c.prepare("SELECT file.oid FROM state JOIN revisions ON revisions.head=state.head JOIN tree_entries folder ON folder.tree=revisions.root JOIN tree_entries file ON file.tree=folder.oid WHERE folder.raw_name=CAST('tasks' AS BLOB)").map_err(unavailable)?.query_map([],|r| r.get(0)).map_err(unavailable)?.collect::<Result<_,_>>().map_err(unavailable)
}

#[test]
fn poisoned_fixture_hooks_fail_closed() {
    let hooks = std::sync::Arc::new(FixtureHooks::default());
    let poisoning = hooks.clone();
    // Poison the actual fixture-owned mutex, not an unrelated surrogate.
    fixture::poison_for_test(&poisoning);
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteProjectBoardStore::open_with_hooks(
        &dir.path().join("poison.sqlite"),
        Identity {
            name: "Poison".into(),
            email: "poison@example.com".into(),
        },
        hooks,
    )
    .unwrap();
    store.bootstrap().unwrap();
    let head = store.read().unwrap().head;
    let before = store.fixture_fingerprint().unwrap();
    assert!(matches!(
        store.fixture_refuse_next_write(),
        Err(BoardStoreError::Unavailable(_))
    ));
    let artifact = crate::application::project_board::ports::BoardArtifact::Run(
        crate::infrastructure::test_support::board_artifacts::run("a", 1),
    );
    assert!(matches!(
        store.write(&head, &[artifact], "board: poisoned"),
        Err(BoardStoreError::Unavailable(_))
    ));
    assert_eq!(store.fixture_fingerprint().unwrap(), before);
}
