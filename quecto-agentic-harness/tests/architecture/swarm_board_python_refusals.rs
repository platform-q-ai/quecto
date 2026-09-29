//! Every refusal the Python board raises has a stable kind (#2303 round-3
//! review L4): [`PYTHON_REFUSALS`] names, for each `SwarmError(...)` text
//! in the Python sources, the [`RefusalKind`] (or, for a text whose kind
//! the board chooses at run time, the kinds) the Rust port records for
//! it. A slice that ports a method (S6–S14) picks its refusals' kinds from
//! this table, and `swarm_board_refusal_kinds`'s site table must agree
//! with it wherever both hold a text.
//!
//! A text is the argument as Python builds it: adjacent literals joined,
//! an f-string's fields kept as written (`{path}`), and any other
//! expression written in braces (`{'; '.join(blockers)}`).
//!
//! Kinds are additive: a new refusal may add a kind, never rename or reuse
//! one (`docs/swarm.md`, Telemetry).
use std::collections::{BTreeMap, BTreeSet};

use quecto::domain::swarm::RefusalKind;

use super::swarm_board_refusal_kinds::REFUSALS;

/// The Python board's sources: the helpers the board runs from, and the
/// policy and use cases they import.
const PYTHON_SOURCES: &[&str] = &[
    "src/application/swarm_use_cases.py",
    "src/domain/swarm_policy.py",
    "src/infrastructure/tools/swarm_helpers/swarm.py",
    "src/infrastructure/tools/swarm_helpers/swarm_repository.py",
    "src/infrastructure/tools/swarm_helpers/swarm_store.py",
    "src/infrastructure/tools/swarm_helpers/swarm_tasks.py",
];

/// `(text, kinds)` for every distinct Python refusal text, sorted by text.
pub(super) const PYTHON_REFUSALS: &[(&str, &[&str])] = &[
    (
        "a deadline is extended only by the supervisor outside the swarm (agent_cmd swarm_control extend)",
        &["supervisor_only"],
    ),
    (
        "a paused run is resumed only by the supervisor outside the swarm (agent_cmd swarm_control resume); members cannot resume it",
        &["supervisor_only"],
    ),
    (
        "a run is closed only by the supervisor outside the swarm (agent_cmd swarm_control close)",
        &["supervisor_only"],
    ),
    ("artifact and revision evidence required", &["invalid"]),
    (
        "completion requires accepted evidence at the current revision for every criterion",
        &["completion_unmet"],
    ),
    ("completion revision required", &["invalid"]),
    ("conflicting launch identity", &["launch_conflict"]),
    ("constraints must be a list of strings", &["invalid"]),
    (
        "coordination request ledger full (10000)",
        &["capacity_full"],
    ),
    ("coordination run missing", &["run_missing"]),
    (
        "coordination store missing at {path}: it was deleted while the run was live, so this run's board is lost",
        &["store_missing"],
    ),
    // The store's failure: contention, a value SQLite cannot bind, or any
    // other failure (`repository.rs`'s `store_kind` and `loose`).
    (
        "coordination store unavailable or contended: {error}",
        &["contended", "invalid", "store"],
    ),
    (
        "criteria distinguish command checks from parent-reviewed requirements",
        &["invalid"],
    ),
    ("cyclic dependencies", &["dependency_cycle"]),
    ("deadline extension must be 1..604800 seconds", &["invalid"]),
    (
        "deadline may be at most seven days ahead, as at creation",
        &["invalid"],
    ),
    ("deadline must be in the next seven days", &["invalid"]),
    (
        "dependencies may change only before claiming",
        &["wrong_state"],
    ),
    ("dependencies must be a bounded list", &["invalid"]),
    ("duplicate criterion id", &["invalid"]),
    (
        "event page requires nonnegative cursor and limit 1 through 100",
        &["invalid"],
    ),
    (
        "evidence must match a configured criterion and kind",
        &["invalid"],
    ),
    (
        "existing live/reserved members exceed requested limit; terminate and reconcile first",
        &["member_limit"],
    ),
    ("exit kind must be orderly or abrupt", &["invalid"]),
    ("explicit evidence criteria required", &["invalid"]),
    (
        "file already reserved: {path}; acquire the entire set or release and retry",
        &["reserved_by_other"],
    ),
    ("file must resolve inside the shared checkout", &["invalid"]),
    (
        "file page requires nonnegative offset and limit 1 through 100",
        &["invalid"],
    ),
    (
        "file reservation board full (1000); release settled work",
        &["capacity_full"],
    ),
    ("invalid non-success outcome", &["invalid"]),
    ("invalid request observation", &["invalid"]),
    ("invalid request usage {field}", &["invalid"]),
    ("invalid, missing or self dependencies", &["invalid"]),
    (
        "invoking member is unknown or death confirmed",
        &["not_member"],
    ),
    (
        "launch reservation does not match invoking process",
        &["launch_conflict"],
    ),
    (
        "member already active in a different process",
        &["launch_conflict"],
    ),
    (
        "member identity already used; choose a stable new identity",
        &["identity_taken"],
    ),
    (
        "member limit must be 1 through 25 including coordinator",
        &["invalid"],
    ),
    ("message id must be a positive integer", &["invalid"]),
    (
        "message {message_id} is already {row['status']}",
        &["wrong_state"],
    ),
    ("new artifact and revision evidence required", &["invalid"]),
    (
        "new artifact evidence must match the revalidated revision",
        &["invalid"],
    ),
    (
        "only a message to the same recipient can be {status}",
        &["invalid"],
    ),
    ("only a paused run may resume", &["wrong_state"]),
    (
        "only abandoned active work can be recovered",
        &["wrong_state"],
    ),
    (
        "only an unlaunched reservation may be released",
        &["wrong_state"],
    ),
    ("only blocked or claimed work may resume", &["wrong_state"]),
    (
        "only claimed, blocked or submitted work can be revoked",
        &["wrong_state"],
    ),
    ("only completed tasks may be revalidated", &["wrong_state"]),
    (
        "only the designated coordinator may do this",
        &["not_coordinator"],
    ),
    (
        "only the setup coordinator can create this run; existing runs cannot be reset",
        &["run_exists"],
    ),
    ("only your own message can be {status}", &["not_owner"]),
    // A pause record the board should hold and does not: a corrupt or
    // outside-edited board.
    ("paused run has no pause record", &["store"]),
    (
        "recipient inbox full (100 unconsumed messages)",
        &["capacity_full"],
    ),
    (
        "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner",
        &["wrong_state"],
    ),
    ("request diagnostic exceeds 32768 bytes", &["invalid"]),
    (
        "request diagnostic ledger full; export before starting another run",
        &["capacity_full"],
    ),
    (
        "request id reused with different payload",
        &["request_id_reused"],
    ),
    (
        "request observation ID reused with different data",
        &["request_id_reused"],
    ),
    (
        "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
        &["wrong_state"],
    ),
    ("reserve 1 through 100 paths together", &["invalid"]),
    // The blockers are the spent budget: a passed deadline or used tokens.
    (
        "resume would pause again at once: {'; '.join(blockers)}",
        &["budget_exhausted"],
    ),
    // Chosen at run time as `policy.rs` chooses it: a spent budget, or a
    // run that is otherwise not running.
    (
        "run already {describe(run)}",
        &["budget_exhausted", "not_running"],
    ),
    (
        "run already {describe(run)}; only the supervisor can resume or close it, and op=cancel_run cancels it",
        &["budget_exhausted", "not_running"],
    ),
    (
        "run is paused (budget-exhausted: deadline); no new work permitted",
        &["budget_exhausted"],
    ),
    (
        "run is {describe(run)}; no new work permitted",
        &["budget_exhausted", "not_running"],
    ),
    (
        "run is {run['status']} without a proposed outcome; resume it or cancel the run",
        &["wrong_state"],
    ),
    (
        "run is {run['status']}; no new admission",
        &["budget_exhausted", "not_running"],
    ),
    (
        "run is {run['status']}; nothing to extend",
        &["not_running"],
    ),
    (
        "run not created yet; nothing to cancel. To start one: swarm op=create",
        &["not_running"],
    ),
    ("run stopped before activation", &["not_running"]),
    (
        "settle outstanding work and file reservations before success",
        &["completion_unmet"],
    ),
    ("stale claim or work not submitted", &["stale_token"]),
    ("stale evidence revision", &["stale_revision"]),
    ("stale launch reservation", &["stale_token"]),
    ("stale or unowned claim", &["stale_token"]),
    (
        "submitted evidence is immutable; release and reclaim before revising",
        &["immutable"],
    ),
    ("summary cursor must be a nonnegative integer", &["invalid"]),
    ("supersedes must be a message id", &["invalid"]),
    (
        "swarm limit {run['member_limit']}, current usage {usage}; reuse the existing pool",
        &["member_limit"],
    ),
    (
        "task acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']",
        &["invalid"],
    ),
    (
        "task board full (1000); settle existing work",
        &["capacity_full"],
    ),
    (
        "task evidence refers to stale revision",
        &["stale_revision"],
    ),
    ("task is not ready to claim", &["wrong_state"]),
    (
        "task page requires nonnegative offset and limit 1 through 100",
        &["invalid"],
    ),
    (
        "token limit must be positive or None, strict_unknown must be boolean",
        &["invalid"],
    ),
    ("unknown message in own inbox", &["not_found"]),
    ("unknown or out-of-swarm recipient", &["not_found"]),
    ("unknown or stale launch reservation", &["stale_token"]),
    ("unknown task", &["not_found"]),
    ("unmet dependencies", &["wrong_state"]),
    ("wake generation is ahead of the board", &["invalid"]),
    (
        "wake generation must be a nonnegative integer",
        &["invalid"],
    ),
    (
        "{label} must be nonempty and at most {maximum} bytes",
        &["invalid"],
    ),
];

/// The argument of the call opening at `open` (just past its `(`), up to
/// its closing parenthesis; strings and nested brackets are skipped.
fn argument(source: &str, open: usize) -> &str {
    let bytes = source.as_bytes();
    let (mut depth, mut quote, mut at) = (0_usize, None, open);
    loop {
        let byte = *bytes.get(at).expect("a SwarmError call is closed");
        match (quote, byte) {
            (Some(_), b'\\') => at += 1,
            (Some(open_quote), byte) if byte == open_quote => quote = None,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'(' | b'[' | b'{') => depth += 1,
            (None, b')') if depth == 0 => return &source[open..at],
            (None, b')' | b']' | b'}') => depth -= 1,
            (None, _) => {}
        }
        at += 1;
    }
}

/// The literal starting at `at` (an optional `f` prefix, then a quote):
/// its text, unescaped, and where it ends.
fn literal(argument: &str, at: usize) -> Option<(String, usize)> {
    let bytes = argument.as_bytes();
    let start = match (bytes.get(at), bytes.get(at + 1)) {
        (Some(b'f'), Some(b'\'' | b'"')) => at + 1,
        (Some(b'\'' | b'"'), _) => at,
        _ => return None,
    };
    let quote = bytes[start];
    let mut text = String::new();
    let mut chars = argument[start + 1..].char_indices();
    while let Some((offset, character)) = chars.next() {
        match character {
            '\\' => text.extend(chars.next().map(|(_, escaped)| escaped)),
            _ if character as u32 == u32::from(quote) => {
                return Some((text, start + 1 + offset + 1));
            }
            _ => text.push(character),
        }
    }
    None
}

/// The end of the expression starting at `at`: the next `+` outside
/// brackets and strings, or the end.
fn expression_end(argument: &str, at: usize) -> usize {
    let bytes = argument.as_bytes();
    let (mut depth, mut quote) = (0_usize, None);
    for (offset, &byte) in bytes[at..].iter().enumerate() {
        match (quote, byte) {
            (Some(open_quote), byte) if byte == open_quote => quote = None,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'(' | b'[' | b'{') => depth += 1,
            (None, b')' | b']' | b'}') => depth -= 1,
            (None, b'+') if depth == 0 => return at + offset,
            (None, _) => {}
        }
    }
    bytes.len()
}

/// The text a `SwarmError` argument builds (see the module docs).
fn text(argument: &str) -> String {
    let mut text = String::new();
    let mut at = 0;
    while at < argument.len() {
        let rest = &argument[at..];
        let skipped = rest.len()
            - rest
                .trim_start_matches(|c: char| c.is_whitespace() || c == '+')
                .len();
        if skipped > 0 {
            at += skipped;
            continue;
        }
        // A literal stands alone only when a `+`, another literal or the
        // end follows it; otherwise it starts an expression (`'; '.join`).
        if let Some((literal, end)) = literal(argument, at) {
            let next = argument[end..].trim_start();
            let alone = next.is_empty() || next.starts_with('+') || literal_starts(next);
            if alone {
                text.push_str(&literal);
                at = end;
                continue;
            }
        }
        let end = expression_end(argument, at);
        text.push('{');
        text.push_str(argument[at..end].trim());
        text.push('}');
        at = end;
    }
    text
}

fn literal_starts(rest: &str) -> bool {
    rest.starts_with(['\'', '"']) || rest.starts_with("f'") || rest.starts_with("f\"")
}

/// Every `SwarmError(...)` text in `source` (its class declaration aside).
fn refusals_in(source: &str) -> Vec<String> {
    const CALL: &str = "SwarmError(";
    source
        .match_indices(CALL)
        .filter(|(at, _)| !source[..*at].ends_with("class "))
        .map(|(at, _)| text(argument(source, at + CALL.len())))
        .collect()
}

fn python_refusals() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for file in PYTHON_SOURCES {
        let source = std::fs::read_to_string(file).unwrap_or_else(|_| panic!("read {file}"));
        let texts = refusals_in(&source);
        assert!(!texts.is_empty(), "{file} raises refusals");
        found.extend(texts);
    }
    found
}

fn kind(name: &str) -> Option<RefusalKind> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

#[test]
fn every_python_refusal_text_has_a_kind() {
    let found = python_refusals();
    assert!(found.len() > 80, "the scan reads the Python board");
    let table: BTreeSet<String> = PYTHON_REFUSALS
        .iter()
        .map(|(text, _)| (*text).to_owned())
        .collect();
    assert_eq!(table.len(), PYTHON_REFUSALS.len(), "one row per text");
    let rows: Vec<String> = found
        .iter()
        .map(|text| format!("    ({text:?}, &[\"…\"]),"))
        .collect();
    assert_eq!(
        found,
        table,
        "the Python refusal table is out of date (one row per text); the sources raise:\n{}",
        rows.join("\n")
    );
    for (text, kinds) in PYTHON_REFUSALS {
        assert!(!kinds.is_empty(), "{text:?} has a kind");
        for name in *kinds {
            let parsed = kind(name).unwrap_or_else(|| panic!("{text:?}: no kind {name}"));
            assert_eq!(parsed.as_str(), *name);
        }
    }
}

/// A text with its fields (`{…}`) blanked, as both tables' templates are
/// compared.
fn blanked(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0_usize;
    for character in text.chars() {
        match character {
            '{' => {
                if depth == 0 {
                    out.push_str("{}");
                }
                depth += 1;
            }
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(character),
            _ => {}
        }
    }
    out
}

/// `RunMissing` → `run_missing`.
fn snake(variant: &str) -> String {
    let mut out = String::new();
    for (index, character) in variant.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(character.to_ascii_lowercase());
    }
    out
}

#[test]
fn the_rust_board_raises_a_python_text_with_its_kind() {
    let python: BTreeMap<String, &[&str]> = PYTHON_REFUSALS
        .iter()
        .map(|(text, kinds)| (blanked(text), *kinds))
        .collect();
    // Two Python texts with one template would let a Rust text match the
    // wrong row (#2303 round-4 review N2).
    assert_eq!(
        python.len(),
        PYTHON_REFUSALS.len(),
        "two Python refusal texts share a template"
    );
    let mut compared = 0;
    for (site, text, variant) in REFUSALS {
        let Some(kinds) = python.get(&blanked(text)) else {
            continue;
        };
        compared += 1;
        match variant.strip_prefix("expr: ") {
            // Chosen at run time: every kind the chooser can return is one
            // the Python row names.
            Some(chooser) => {
                let chosen = chosen_kinds(site, chooser);
                assert!(!chosen.is_empty(), "{site}: {chooser} names no kind");
                for kind in &chosen {
                    assert!(
                        kinds.contains(&kind.as_str()),
                        "{site}: {text:?} may be {kind} in Rust, {kinds:?} in the Python table"
                    );
                }
            }
            None => assert!(
                kinds.contains(&snake(variant).as_str()),
                "{site}: {text:?} is {variant} in Rust, {kinds:?} in the Python table"
            ),
        }
    }
    assert!(compared > 20, "the two tables share texts ({compared})");
}

/// The kinds a run-time chooser can return: every `RefusalKind::X` the
/// function it calls (the expression's leading name, `not_running (…)`)
/// names, in the site's file (`file:function`), as snake case.
fn chosen_kinds(site: &str, chooser: &str) -> BTreeSet<String> {
    let (file, _) = site.split_once(':').expect("a site is file:function");
    let source = std::fs::read_to_string(file).unwrap_or_else(|_| panic!("read {file}"));
    kinds_chosen_in(&source, chooser)
}

fn kinds_chosen_in(source: &str, chooser: &str) -> BTreeSet<String> {
    let name = chooser
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default();
    let file = syn::parse_file(source).expect("a crate source parses");
    let function = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == name => Some(function),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no chooser fn {name} for {chooser:?}"));
    let body = quote::ToTokens::to_token_stream(&function.block).to_string();
    let words: Vec<&str> = body.split_whitespace().collect();
    words
        .windows(3)
        .filter(|window| window[0] == "RefusalKind" && window[1] == "::")
        .map(|window| snake(window[2]))
        .collect()
}

#[test]
fn a_chooser_returns_the_kinds_its_function_names() {
    let chosen = kinds_chosen_in(
        "fn other() -> RefusalKind { RefusalKind::Store }\n\
         fn not_running(spent: bool) -> RefusalKind {\n\
             match spent { true => RefusalKind::BudgetExhausted, false => RefusalKind::NotRunning }\n\
         }",
        "not_running (budget_spent (run))",
    );
    assert_eq!(
        chosen.into_iter().collect::<Vec<_>>(),
        ["budget_exhausted", "not_running"]
    );
    let policy = chosen_kinds(
        "src/domain/swarm/policy.rs:admission",
        "not_running (budget_spent (run) || expired (run , now))",
    );
    assert_eq!(
        policy.into_iter().collect::<Vec<_>>(),
        ["budget_exhausted", "not_running"]
    );
}

#[test]
fn the_scan_reads_literals_f_strings_and_expressions() {
    let found = refusals_in(
        r#"
class SwarmError(RuntimeError):
    pass
raise SwarmError('plain')
raise SwarmError(f"run is {describe(run)}; no")
raise SwarmError(
    f'missing at {path}: gone, '
    'so this run\'s board is lost') from error
raise SwarmError('again: ' + '; '.join(blockers))
raise SwarmError(f"message {m} is already {row['status']}")
raise SwarmError('line\nnext \\ back \' single \" double \tab')
"#,
    );
    assert_eq!(
        found,
        [
            "plain",
            "run is {describe(run)}; no",
            "missing at {path}: gone, so this run's board is lost",
            "again: {'; '.join(blockers)}",
            "message {m} is already {row['status']}",
            "line\nnext \\ back ' single \" double \tab",
        ]
    );
    assert_eq!(blanked("run is {describe(run)}; no"), "run is {}; no");
    assert_eq!(snake("NotRunning"), "not_running");
}
