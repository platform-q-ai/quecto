use super::*;

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
        put(&tx, "tasks/partial.md", b"uncommitted bytes").unwrap();
        store.advance(&tx).unwrap();
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
    assert!(rows(&store.connection.lock().unwrap()).unwrap().is_empty());
    let c = store.connection.lock().unwrap();
    let revisions: i64 = c
        .query_row("SELECT count(*) FROM revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(revisions, 1, "uncommitted provenance also rolled back");
}
