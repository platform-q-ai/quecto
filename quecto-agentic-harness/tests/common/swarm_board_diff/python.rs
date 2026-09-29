//! The Python side of the differential harness: one `python3 -I` child
//! running the board's **current** `.py` sources (compiled in with
//! `include_str!`, as `swarm_bridge::bootstrap_source` does), so every
//! Python edit is covered until S18 deletes them.
//!
//! The child keeps one `Workbench` per member. Before each call it sets the
//! step's clock on the `time.time` module attribute (every module shares
//! it: `create` validates on it and `_end_by_loss` stamps with it) and on
//! the member's `coordination.clock`, and `uuid.uuid4` draws from a counter
//! whose `.hex` is `format(n, '032x')`, the sequence `rust.rs` draws too.
//!
//! Protocol: one JSON line `[member, method, args, now]` in, where `args`
//! is the step's argument text as a JSON string, which the driver parses
//! with `json.loads` (so Python reads the text the step wrote, not a
//! re-serialization of it); one line out,
//! `{"ok": result}`, `{"error": text}` for a `SwarmError`, or
//! `{"exception": text}` for anything else. Driver-only aliases stand in for
//! the Rust dispatcher's test names (`create_run` is `create` without its
//! closing summary, `bootstrap_run` is `_bootstrap` without its join, and
//! `bootstrap_join` is that join, `join_process`, bound by `_bootstrap`'s
//! signature and without the coordinator's closing summary, and
//! `task_raw` is `Tasks._task` inside a read-only operation, without the
//! owner liveness `task` adds); they live here, never in the `.py`
//! sources.
//!
//! `create_run` stops where Python's real `create` commits: `create`'s
//! transaction commits and only then does it call `summary()`, which can
//! still raise, so a real `create` can answer a refusal for a run it has
//! created. The Rust `create` keeps that order and that outcome (#2277):
//! committed, then refused.
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use quecto::infrastructure::persistence::swarm_board::py_json;
use serde_json::{Value, json};

use super::Outcome;

/// How long one board call may take before the child is presumed hung.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

const SOURCES: [(&str, &str); 6] = [
    (
        "swarm_policy",
        include_str!("../../../src/domain/swarm_policy.py"),
    ),
    (
        "swarm_use_cases",
        include_str!("../../../src/application/swarm_use_cases.py"),
    ),
    (
        "swarm_repository",
        include_str!("../../../src/infrastructure/tools/swarm_helpers/swarm_repository.py"),
    ),
    (
        "swarm_store",
        include_str!("../../../src/infrastructure/tools/swarm_helpers/swarm_store.py"),
    ),
    (
        "swarm_tasks",
        include_str!("../../../src/infrastructure/tools/swarm_helpers/swarm_tasks.py"),
    ),
    (
        "swarm",
        include_str!("../../../src/infrastructure/tools/swarm_helpers/swarm.py"),
    ),
];

const DRIVER: &str = r#"
import swarm
from swarm_policy import SwarmError

_now = [0.0]
time.time = lambda: _now[0]

class _Uuid:
    __slots__ = ('hex',)
    def __init__(self, n):
        self.hex = format(n, '032x')

_drawn = [0]
def _uuid4():
    _drawn[0] += 1
    return _Uuid(_drawn[0])
uuid.uuid4 = _uuid4

DATABASE, CHECKOUT = sys.argv[1], sys.argv[2]
_boards = {}

def _board(member):
    board = _boards.get(member)
    if board is None:
        board = _boards[member] = swarm.Workbench(DATABASE, CHECKOUT, member)
    board.coordination.clock = lambda: _now[0]
    return board

def _invoke(method, args):
    return method(*args) if isinstance(args, list) else method(**args)

def _create_run(board, args):
    board.summary = lambda *a, **k: None
    try:
        _invoke(board.create, args)
    finally:
        del board.summary

def _bootstrap_run(board, args):
    board._join = lambda *a, **k: None
    try:
        _invoke(board._bootstrap, args)
    finally:
        del board._join

def _bootstrap_join(board, args):
    def join(pid, started, socket, reservation=None):
        return swarm.join_process(board, reservation, pid, started, socket)
    saved = swarm.Workbench.summary
    swarm.Workbench.summary = lambda self, *a, **k: None
    try:
        _invoke(join, args)
    finally:
        swarm.Workbench.summary = saved

def _task_raw(board, args):
    def raw(task_id):
        with board.store.operation(active=False, read_only=True) as (db, _):
            return board._task(db, task_id)
    return _invoke(raw, args)

_ALIASES = {'create_run': _create_run, 'bootstrap_run': _bootstrap_run, 'bootstrap_join': _bootstrap_join,
            'task_raw': _task_raw}

for _line in sys.stdin:
    _member, _method, _args_text, _step_now = json.loads(_line)
    _now[0] = float(_step_now)
    _target = _board(_member)
    try:
        _args = json.loads(_args_text)
        if _method in _ALIASES:
            _out = {'ok': _ALIASES[_method](_target, _args)}
        else:
            _out = {'ok': _invoke(getattr(_target, _method), _args)}
    except SwarmError as error:
        _out = {'error': str(error)}
    except Exception as error:
        _out = {'exception': type(error).__name__ + ': ' + str(error)}
    try:
        _line = json.dumps(_out)
    except Exception as error:
        _line = json.dumps({'exception': 'unwritable result: ' + type(error).__name__ + ': ' + str(error)})
    sys.stdout.write(_line + '\n')
    sys.stdout.flush()
"#;

/// The driver program: the six modules registered under their names, then
/// the call loop.
fn program() -> String {
    let mut program = String::from("import sys, types, json, time, uuid\n");
    for (name, body) in SOURCES {
        program.push_str(&format!(
            "_m=types.ModuleType({name:?}); sys.modules[{name:?}]=_m; exec(compile({}, {name:?}, 'exec'), _m.__dict__)\n",
            serde_json::to_string(body).expect("source serializes")
        ));
    }
    program.push_str(DRIVER);
    program
}

/// The methods of Python's `Workbench` defined in the board's own modules
/// (`swarm.py`, `swarm_tasks.py`), public and underscore, dunders left
/// out: `dir(Workbench)` filtered by callables whose `__module__` is one
/// of them, sorted.
pub fn workbench_methods() -> Vec<String> {
    let mut program = String::from("import sys, types, json, time, uuid\n");
    for (name, body) in SOURCES {
        program.push_str(&format!(
            "_m=types.ModuleType({name:?}); sys.modules[{name:?}]=_m; exec(compile({}, {name:?}, 'exec'), _m.__dict__)\n",
            serde_json::to_string(body).expect("source serializes")
        ));
    }
    program.push_str(
        "import swarm\n\
         _w = swarm.Workbench\n\
         print(json.dumps(sorted(n for n in dir(_w) if not n.startswith('__') \
         and callable(getattr(_w, n)) \
         and getattr(getattr(_w, n), '__module__', None) in ('swarm', 'swarm_tasks'))))\n",
    );
    let output = Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(program)
        .stderr(Stdio::inherit())
        .output()
        .expect("python3 is required for the differential harness");
    assert!(output.status.success(), "the Workbench listing ran");
    serde_json::from_slice(&output.stdout).expect("a JSON list of method names")
}

pub struct PyBoard {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<std::io::Result<String>>,
}

impl PyBoard {
    /// Starts the child for the board file `database` of `checkout`; the
    /// driver program is written into `workdir`.
    pub fn start(database: &Path, checkout: &Path, workdir: &Path) -> Self {
        let script = workdir.join("board_driver.py");
        std::fs::write(&script, program()).expect("write the Python driver");
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg(database)
            .arg(checkout)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 is required for the differential harness");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        let stdin = child.stdin.take();
        Self {
            child,
            stdin,
            lines,
        }
    }

    /// One board call as `member` at `now`.
    /// `args` is JSON text, parsed on the Python side.
    pub fn call(&mut self, member: &str, method: &str, args: &str, now: f64) -> Outcome {
        let request = json!([member, method, args, now]).to_string();
        let stdin = self.stdin.as_mut().expect("the driver's stdin is open");
        writeln!(stdin, "{request}").expect("send a call to the Python driver");
        stdin.flush().expect("flush the call");
        let line = match self.lines.recv_timeout(CALL_TIMEOUT) {
            Ok(line) => line.expect("read the Python driver's answer"),
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.child.kill();
                panic!("the Python driver did not answer {request} within {CALL_TIMEOUT:?}");
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the Python driver exited before answering {request}")
            }
        };
        let answer = python_answer(&line);
        match (
            answer.get("ok"),
            answer.get("error"),
            answer.get("exception"),
        ) {
            (Some(result), None, None) => Outcome::Ok(result.clone()),
            (None, Some(Value::String(text)), None) => Outcome::Refused(text.clone()),
            (None, None, Some(Value::String(text))) => Outcome::Raised(text.clone()),
            _ => panic!("the driver answered an unknown shape: {line}"),
        }
    }
}

/// The driver's answer line, read as Python's `json.loads` reads it
/// (#2277 review M1): `py_json`'s float parse is correctly rounded, so
/// every float compares as the exact double Python answered, where
/// `serde_json` without `float_roundtrip` can misread its last digit. A
/// value no `serde_json::Value` holds (`NaN`, `Infinity`, an integer
/// beyond u64, a lone surrogate) is no answer the harness compares, and
/// panics.
pub fn python_answer(line: &str) -> Value {
    py_json::decode(line)
        .and_then(|answer| answer.to_value())
        .unwrap_or_else(|error| panic!("the driver answered {line:?}: {error}"))
}

impl Drop for PyBoard {
    fn drop(&mut self) {
        // Closing stdin ends the driver's read loop; reap it so no python3
        // outlives the harness.
        self.stdin.take();
        let _ = self.child.wait();
    }
}
