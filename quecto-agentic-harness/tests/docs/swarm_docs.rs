//! #2280 (E1-S15): the swarm operator manual (`docs/swarm.md`) teaches the
//! structured board ops over the in-process Rust board, maps every member op,
//! documents the board telemetry for the improvement loops, and the
//! subagents manual's process-effect table is the architecture allowlist.
use std::collections::BTreeSet;

use quecto::infrastructure::tools::swarm_board_ops::BOARD_OPS;

use crate::common::read_repo_file;

fn swarm_doc() -> String {
    read_repo_file("docs/swarm.md")
}

/// `board.` followed by a method name: a Python board call.
fn python_board_calls(doc: &str) -> Vec<String> {
    doc.lines()
        .filter(|line| {
            line.match_indices("board.").any(|(index, _)| {
                line[index + "board.".len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
            })
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn python_board_calls_are_told_from_prose() {
    assert_eq!(
        python_board_calls("the board.\nboard.claim(3)\nboard.<method>\nthe board. Next"),
        vec!["board.claim(3)".to_owned()]
    );
}

#[test]
fn the_swarm_doc_teaches_only_structured_ops() {
    let doc = swarm_doc();
    for retired in [
        "from swarm import",
        "op=run",
        "\"op\":\"run\"",
        "persistent interpreter",
        "python3 -I",
    ] {
        assert!(
            !doc.contains(retired),
            "docs/swarm.md still teaches `{retired}`"
        );
    }
    let calls = python_board_calls(&doc);
    assert!(
        calls.is_empty(),
        "Python board calls:\n{}",
        calls.join("\n")
    );
    // `tests/swarm_helpers_test.py` pins the idle threshold in this prose
    // until the Python board goes (S18).
    assert!(doc.contains("300 seconds"));
    assert!(doc.contains("ADR-0030"));
}

/// The API section is the mapping table: one row per member op, naming the
/// op and every argument it takes.
#[test]
fn the_swarm_doc_maps_every_member_op() {
    let doc = swarm_doc();
    let mut missing = Vec::new();
    for spec in BOARD_OPS {
        let cell = format!("| `{}` |", spec.name);
        match doc.lines().find(|line| line.starts_with(&cell)) {
            Some(row) => {
                for arg in spec.args {
                    if !row.contains(&format!("`{}`", arg.name)) {
                        missing.push(format!("{}: argument {}", spec.name, arg.name));
                    }
                }
            }
            None => missing.push(format!("{}: no mapping row", spec.name)),
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

/// The owner's improvement loops read the board telemetry: the doc says how
/// to switch it on, what each record and tracing event carries, and where
/// to find a run's records.
#[test]
fn the_swarm_doc_explains_the_board_telemetry() {
    let doc = swarm_doc();
    let telemetry = doc
        .split_once("## Telemetry")
        .map(|(_, section)| section)
        .expect("docs/swarm.md has a Telemetry section");
    for needle in [
        "\"telemetry\": {\"event_log\": {\"enabled\": true}}",
        "<base_dir>/audit/",
        "\"event\":\"swarm_op\"",
        "\"run_id\"",
        "quecto::swarm_board",
        "RUST_LOG=quecto::swarm_board=debug",
        "swarm structured op",
        "`gate`",
        "`lifecycle`",
        "`warnings`",
        "`swarm_ops_dropped`",
        "DEBUG",
        "INFO",
    ] {
        assert!(
            telemetry.contains(needle),
            "the Telemetry section misses {needle}"
        );
    }
}

/// S14's member-input rule is documented for operators too.
#[test]
fn the_swarm_doc_states_the_member_input_rule() {
    let doc = swarm_doc();
    for needle in [
        "i64",
        "NaN",
        "lone surrogate",
        "arguments: not representable",
    ] {
        assert!(doc.contains(needle), "docs/swarm.md misses {needle}");
    }
}

/// The files of `PROCESS_EFFECT_ALLOWLIST` in the architecture test, with
/// their `src/` prefix dropped.
fn allowlisted_files() -> BTreeSet<String> {
    let source = read_repo_file("tests/architecture/teardown_authority.rs");
    let (_, rest) = source
        .split_once("const PROCESS_EFFECT_ALLOWLIST")
        .expect("the allowlist");
    let (block, _) = rest.split_once("\n];").expect("the allowlist's end");
    let files: BTreeSet<String> = block
        .lines()
        .filter_map(|line| line.trim().strip_prefix("\"src/"))
        .filter_map(|line| line.split_once('"').map(|(path, _)| path.to_owned()))
        .collect();
    assert!(files.len() >= 10, "{files:?}");
    files
}

/// The backticked `.rs` paths the table rows of `docs/subagents.md`'s
/// allowlist section name.
fn documented_files() -> BTreeSet<String> {
    let doc = read_repo_file("docs/subagents.md");
    let (_, section) = doc
        .split_once("#### The allowlist boundary")
        .expect("the allowlist section");
    section
        .lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .flat_map(|row| {
            row.split('`')
                .skip(1)
                .step_by(2)
                .filter(|token| token.ends_with(".rs"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_subagents_allowlist_table_is_the_architecture_allowlist() {
    assert_eq!(documented_files(), allowlisted_files());
}
