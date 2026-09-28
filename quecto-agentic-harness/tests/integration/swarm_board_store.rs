//! The Rust board store and the Python `swarm_store.Store` share one board
//! file (#2269, owner decision D2): each one's `BEGIN IMMEDIATE`
//! transaction excludes the other with the same contended message after
//! the same 500 ms busy timeout, and a board Python created opens in Rust
//! without any change to its schema.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use quecto::infrastructure::persistence::swarm_board::store::{BoardStore, StoreRefusal};
use serial_test::serial;

const SOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

/// Drives `swarm_store.Store` on the board at `argv[2]`: `create` creates
/// it; `hold` holds a transaction, printing `held`, until a line arrives on
/// stdin (a 60 s alarm ends it if none does); `try` runs one empty
/// transaction and prints `ok`, or `refused`, the seconds it waited and the
/// `SwarmError` text, tab-separated.
const PYTHON_STORE: &str = "import signal, sys, time
root = sys.argv[3]
for part in ('infrastructure/tools/swarm_helpers', 'domain', 'application'):
    sys.path.insert(0, root + '/' + part)
from swarm_store import Store
from swarm_policy import SwarmError
mode, path = sys.argv[1], sys.argv[2]
store = Store(path, 'python')
if mode == 'create':
    with store.transaction(create=True):
        pass
    print('created')
elif mode == 'hold':
    signal.alarm(60)
    with store.transaction():
        print('held', flush=True)
        sys.stdin.readline()
    print('released')
else:
    started = time.monotonic()
    try:
        with store.transaction():
            pass
        print('ok')
    except SwarmError as error:
        print('refused', time.monotonic() - started, error, sep='\\t')
";

fn python(mode: &str, board: &Path) -> Command {
    let mut command = Command::new("python3");
    command
        .args(["-I", "-c", PYTHON_STORE, mode])
        .arg(board)
        .arg(SOURCES)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stderr(Stdio::inherit());
    command
}

fn run_python(mode: &str, board: &Path) -> String {
    let output = python(mode, board)
        .stdin(Stdio::null())
        .output()
        .expect("python3 is required for the mixed-writer test");
    assert!(output.status.success(), "python3 {mode} failed");
    String::from_utf8(output.stdout)
        .expect("python prints text")
        .trim_end()
        .to_owned()
}

fn read_line(reader: &mut BufReader<ChildStdout>) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).expect("python's output reads");
    line.trim_end().to_owned()
}

fn finish(mut child: Child, reader: &mut BufReader<ChildStdout>) -> String {
    child
        .stdin
        .take()
        .expect("python's stdin is piped")
        .write_all(b"release\n")
        .expect("python is told to release");
    let last = read_line(reader);
    assert!(
        child.wait().expect("python exits").success(),
        "python exits cleanly"
    );
    last
}

fn master(board: &Path) -> Vec<(String, String, String, Option<String>)> {
    let connection = rusqlite::Connection::open(board).expect("the board opens");
    let mut statement = connection
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY name")
        .expect("sqlite_master is readable");
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("sqlite_master rows")
        .collect::<Result<_, _>>()
        .expect("sqlite_master rows read")
}

#[test]
#[serial]
fn python_and_rust_transactions_exclude_each_other_on_one_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let board = dir.path().join("swarm.sqlite");
    let store = BoardStore::new(&board);
    store
        .transaction(true, |_| Ok(()))
        .expect("Rust creates the board");

    // Python holds the write lock; Rust waits the busy timeout, then refuses.
    let mut holder = python("hold", &board)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("python3 is required for the mixed-writer test");
    let mut reader = BufReader::new(holder.stdout.take().expect("python's stdout is piped"));
    assert_eq!(read_line(&mut reader), "held");
    let started = Instant::now();
    let refused = store.transaction(false, |_| Ok(()));
    let waited = started.elapsed();
    assert_eq!(finish(holder, &mut reader), "released");
    assert_eq!(
        refused,
        Err(StoreRefusal(
            "coordination store unavailable or contended: database is locked".into()
        ))
    );
    assert!(
        waited >= Duration::from_millis(450),
        "Rust waited the busy timeout: {waited:?}"
    );
    assert!(waited < Duration::from_secs(5), "then gave up: {waited:?}");

    // Rust holds the write lock; Python refuses with the identical text.
    let python_try = store
        .transaction(false, |_| Ok(run_python("try", &board)))
        .expect("Rust holds and commits");
    let fields: Vec<&str> = python_try.splitn(3, '\t').collect();
    assert_eq!(fields.first(), Some(&"refused"), "{python_try}");
    assert_eq!(
        fields.get(2),
        Some(&"coordination store unavailable or contended: database is locked")
    );
    let python_waited: f64 = fields
        .get(1)
        .and_then(|seconds| seconds.parse().ok())
        .expect("seconds waited");
    assert!(
        python_waited >= 0.45,
        "Python waited the busy timeout: {python_waited}"
    );

    // Released, each takes its turn.
    assert_eq!(run_python("try", &board), "ok");
    store
        .transaction(false, |_| Ok(()))
        .expect("Rust opens after Python");
}

#[test]
#[serial]
fn a_python_created_board_opens_in_rust_with_no_schema_change() {
    let dir = tempfile::tempdir().expect("temp dir");
    let board = dir.path().join("swarm.sqlite");
    assert_eq!(run_python("create", &board), "created");
    let before = master(&board);
    let store = BoardStore::new(&board);
    store
        .transaction(false, |_| Ok(()))
        .expect("Rust opens Python's board");
    store
        .transaction(true, |_| Ok(()))
        .expect("and a Rust create is a no-op");
    assert_eq!(master(&board), before);
    let journal: String = rusqlite::Connection::open(&board)
        .expect("the board opens")
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode reads");
    assert_eq!(journal, "delete");
}
