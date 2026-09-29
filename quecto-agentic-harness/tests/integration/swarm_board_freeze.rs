//! The one-shot freeze of the Python board's answers (#2283), run by hand
//! while the Python board still existed, in the commit that recorded them.
//! Each test here does its work only when `QUECTO_SWARM_FREEZE=1` asks for
//! it (the pre-commit gate refuses an ignored test), and is otherwise a no-op:
//!
//! ```text
//! QUECTO_SWARM_FREEZE=1 cargo test --features test-support --test integration \
//!   -- --test-threads 1 freeze_legacy_python_board freeze_json_corpus_golden
//! QUECTO_SWARM_FREEZE=1 cargo test --features test-support --test integration \
//!   -- --exact swarm_board_freeze::freeze_differential_goldens
//! QUECTO_SWARM_FREEZE=1 cargo test --features test-support --test integration \
//!   -- the_goldens_match_the_live_python_board
//! ```
//!
//! - `freeze_differential_goldens` records every differential scenario's
//!   golden fixture on the live Python board, twice, into two scratch
//!   folders (re-running this test binary with `QUECTO_SWARM_GOLDEN=record`
//!   and the differential tests only), requires the two recordings to be
//!   byte-identical (the Python board is deterministic under the harness's
//!   clock, id counter and fixed hash seed), and installs one as the
//!   committed fixtures.
//! - `freeze_json_corpus_golden` writes Python's texts for the codec's
//!   corpus; `freeze_legacy_python_board` writes the legacy board.
//! - `the_goldens_match_the_live_python_board` re-runs every scenario on
//!   the live Python board (`QUECTO_SWARM_GOLDEN=verify`) and requires the
//!   committed fixtures to be exactly what it answers, and checks the other
//!   frozen answers against it too.
//!
//! The commit that deletes the Python board deletes this file with it.
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::golden::GOLDEN_DIR;
use crate::swarm_board_diff_runs::swarm_board_diff::python::{
    PyBoard, workbench_methods, workbench_parameters,
};

/// The tests whose scenarios hold goldens.
const DIFFERENTIAL: &str = "swarm_board_";

/// Whether `QUECTO_SWARM_FREEZE=1` asks for the freeze.
fn freezing() -> bool {
    matches!(std::env::var("QUECTO_SWARM_FREEZE").as_deref(), Ok("1"))
}

/// Re-runs this test binary's `swarm_board_` tests with `envs` (these
/// freeze tests among them, as no-ops); they must pass.
fn rerun(envs: &[(&str, &Path)], mode: &str) {
    let status = Command::new(std::env::current_exe().expect("the test binary"))
        .arg(DIFFERENTIAL)
        .env_remove("QUECTO_SWARM_FREEZE")
        .env("QUECTO_SWARM_GOLDEN", mode)
        .envs(envs.iter().map(|(name, value)| (*name, *value)))
        .stdin(std::process::Stdio::null())
        .status()
        .expect("re-run the test binary");
    assert!(status.success(), "the {mode} run passed");
}

/// Every file under `dir`, relative, with its bytes, sorted.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).expect("read a golden folder") {
            let path = entry.expect("a golden entry").path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("read a golden");
                files.push((path.strip_prefix(dir).unwrap().to_path_buf(), bytes));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn freeze_differential_goldens() {
    if !freezing() {
        return;
    }
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (first, second) = (scratch.path().join("first"), scratch.path().join("second"));
    rerun(&[("QUECTO_SWARM_GOLDEN_DIR", &first)], "record");
    rerun(&[("QUECTO_SWARM_GOLDEN_DIR", &second)], "record");
    let recorded = tree(&first);
    assert!(
        recorded.len() > 300,
        "every scenario recorded: {}",
        recorded.len()
    );
    assert!(
        recorded == tree(&second),
        "two recordings on the live Python board are byte-identical"
    );
    let committed = Path::new(GOLDEN_DIR);
    if committed.exists() {
        std::fs::remove_dir_all(committed).expect("clear the old goldens");
    }
    for (path, bytes) in &recorded {
        let target = committed.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).expect("a golden folder");
        std::fs::write(&target, bytes).expect("install a golden");
    }
    eprintln!("installed {} golden fixtures", recorded.len());
}

#[test]
fn the_goldens_match_the_live_python_board() {
    if !freezing() {
        return;
    }
    rerun(&[], "verify");
    let frozen: Vec<String> = crate::swarm_board_diff_reads_calls::WORKBENCH_METHODS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(frozen, workbench_methods(), "the frozen Workbench methods");
    for (method, parameters) in crate::swarm_board_diff_reads_calls::WORKBENCH_PARAMETERS {
        assert_eq!(
            parameters
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
            workbench_parameters(method),
            "{method}'s frozen parameters"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let mut python = PyBoard::start(&dir.path().join("swarm.sqlite"), dir.path(), dir.path());
    assert_eq!(
        python.call("parent", "tasks", r#"{"x": 1, "y": 2}"#, 0.0),
        Outcome::Raised(crate::swarm_board_diff_wire::PYTHON_TWO_UNEXPECTED.to_owned())
    );
    assert_eq!(
        python_corpus_lines(),
        crate::swarm_board_py_json::golden_lines(),
        "the frozen corpus texts"
    );
}

/// The corpus texts `python3` writes (see `swarm_board_py_json`).
const PYTHON_WRITER: &str = "import json, sys
assert sys.version_info >= (3, 13), 'the codec reproduces CPython 3.13 messages'
corpus = json.load(open(sys.argv[1], encoding='utf-8'))
values = corpus['values'] + [json.loads(text) for text in corpus['texts']]
for value in values:
    print(json.dumps(value, sort_keys=True, separators=(',', ':')))
    print(json.dumps(value))
for text in corpus['invalid']:
    try:
        json.loads(text)
        print('accepted')
    except (ValueError, RecursionError) as error:
        print('refused: ' + str(error))
";

fn python_corpus_lines() -> Vec<String> {
    let output = Command::new("python3")
        .args([
            "-I",
            "-c",
            PYTHON_WRITER,
            crate::swarm_board_py_json::CORPUS,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("python3 wrote the corpus");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("ensure_ascii output is ASCII")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn freeze_json_corpus_golden() {
    if !freezing() {
        return;
    }
    let version = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"])
        .output()
        .expect("python3's version");
    let golden = json!({
        "about": "What CPython's json.dumps (sort_keys=True, separators=(',', ':')), plain json.dumps and json.loads wrote for json_corpus.json, frozen when the Python board was deleted (#2283): per value, then per text's value, its encode text and its dumps text; then per invalid text 'refused: ' and Python's message.",
        "cpython": String::from_utf8_lossy(&version.stdout).trim(),
        "lines": python_corpus_lines(),
    });
    let mut text = serde_json::to_string_pretty(&golden).unwrap();
    text.push('\n');
    std::fs::write(crate::swarm_board_py_json::GOLDEN, text).expect("write the corpus golden");
}

#[test]
fn freeze_legacy_python_board() {
    if !freezing() {
        return;
    }
    use crate::swarm_board_legacy::{LEGACY_BOARD, python_steps, with_tokens};
    let dir = tempfile::tempdir().expect("a checkout");
    let database = dir.path().join("swarm.sqlite");
    {
        let mut python = PyBoard::start(&database, dir.path(), dir.path());
        let mut tokens: Vec<(String, String)> = Vec::new();
        for crate::swarm_board_legacy::Call(member, method, args, offset) in python_steps() {
            let args = with_tokens(&args, &tokens);
            let now = crate::swarm_board_diff_runs::NOW + offset;
            let outcome = python.call(member, method, &args.to_string(), now);
            let Outcome::Ok(answer) = outcome else {
                panic!("{member} {method} {args}: {outcome:?}");
            };
            if method == "claim" {
                let token = answer["token"].as_str().expect("a claim token").to_owned();
                tokens.push((format!("token{}", args[0]), token));
            }
            let _: &Value = &answer;
        }
    }
    assert!(
        !dir.path().join("swarm.sqlite-journal").exists(),
        "the Python board left no journal"
    );
    std::fs::copy(&database, LEGACY_BOARD).expect("install the legacy board");
}
