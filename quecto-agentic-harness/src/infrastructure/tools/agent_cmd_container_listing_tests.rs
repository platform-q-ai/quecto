//! #2220: `get_containers` is bounded and compact.

use std::sync::Arc;

use super::*;
use crate::application::environments::use_cases::ListEnvironmentsQuery;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environments::services::environment_listing::tests::{
    CALLER, qa_record, qa_registry, quiet_journal,
};
use crate::domain::environments::services::environment_listing::{
    EnvironmentListing, ListedEnvironment,
};
use crate::domain::tool_policy::value_objects::tool::ToolResult;

fn get_containers(registry: EnvironmentRegistry, args: serde_json::Value) -> ToolResult {
    let query = Arc::new(ListEnvironmentsQuery::new(registry));
    let mut call = serde_json::json!({"agent_id": "*", "command": "get_containers"});
    for (key, value) in args.as_object().expect("args are an object") {
        call[key] = value.clone();
    }
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(
            super::super::agent_cmd_containers::execute_container_command(
                Some(&query),
                None,
                None,
                &call,
            ),
        )
}

fn listed(registry: EnvironmentRegistry, args: serde_json::Value) -> (usize, serde_json::Value) {
    let result = get_containers(registry, args);
    assert!(!result.is_error, "{}", result.content);
    (
        result.content.len(),
        serde_json::from_str(&result.content).expect("the listing is JSON"),
    )
}

fn refs(listing: &serde_json::Value) -> Vec<String> {
    listing["containers"]
        .as_array()
        .expect("containers")
        .iter()
        .map(|row| row["ref"].as_str().expect("ref").to_string())
        .collect()
}

#[test]
fn the_2220_registry_lists_only_what_the_caller_can_act_on_compactly() {
    let (bytes, listing) = listed(qa_registry(), serde_json::json!({}));
    eprintln!("#2220 default listing: {bytes} bytes: {listing}");
    assert!(bytes < 1_200, "{bytes} bytes: {listing}");
    assert_eq!(refs(&listing), ["C13", "C14", "C15", "C16"]);
    assert_eq!(listing["total"], 16);
    assert_eq!(listing["hidden"], 12);
    assert!(listing.get("omitted").is_none(), "{listing}");
    assert_eq!(
        listing["note"],
        "12 not joinable (stopped, or other sessions'); \"all\":true lists them, also at most 20"
    );
    assert_eq!(
        listing["repository"],
        "https://github.com/platform-q-ai/quecto.git"
    );
    let workspace = "/home/qa/.quecto/containers/standard/state/env-q000000016/workspace";
    assert_eq!(
        listing["containers"][3],
        serde_json::json!({
            "ref": "C16",
            "status": "running",
            "config": "standard",
            "members": 1,
            "own": true,
            "checkout": format!("{workspace}/repo"),
            "created_at": 1_790_000_960u64,
        })
    );
    assert_eq!(
        listing["containers"][0],
        serde_json::json!({
            "ref": "C13",
            "status": "empty",
            "config": "standard",
            "members": 0,
            "own": false,
            "session": "cli:qa5-h-2",
            "restored": true,
            "checkout": "/home/qa/.quecto/containers/standard/state/env-q000000013/workspace/repo",
            "created_at": 1_790_000_780u64,
        })
    );
}

#[test]
fn all_lists_every_environment_compactly_in_ref_number_order() {
    let (bytes, listing) = listed(qa_registry(), serde_json::json!({"all": true}));
    eprintln!("#2220 all listing: {bytes} bytes");
    assert!(bytes < 5_000, "{bytes} bytes: {listing}");
    let expected: Vec<String> = (1..=16).map(|n| format!("C{n}")).collect();
    assert_eq!(refs(&listing), expected);
    assert_eq!(listing["total"], 16);
    assert!(listing.get("hidden").is_none(), "{listing}");
    assert!(listing.get("note").is_none(), "{listing}");
    assert_eq!(listing["containers"][1]["status"], "stopped");
    assert_eq!(
        listing["containers"][1]["last_error"],
        "container not found at restore: the runtime reports it gone"
    );
    for row in listing["containers"].as_array().unwrap() {
        for dropped in ["metadata", "environment_uuid", "workspace", "repository"] {
            assert!(row.get(dropped).is_none(), "{dropped} in {row}");
        }
    }
}

#[test]
fn all_must_be_a_boolean_and_null_or_false_is_the_default() {
    for all in [serde_json::json!("yes"), serde_json::json!(1)] {
        let result = get_containers(qa_registry(), serde_json::json!({"all": all}));
        assert!(result.is_error, "{}", result.content);
        assert!(
            result.content.contains("all must be a boolean"),
            "{}",
            result.content
        );
    }
    for all in [serde_json::Value::Null, serde_json::json!(false)] {
        let (_, listing) = listed(qa_registry(), serde_json::json!({"all": all}));
        assert_eq!(refs(&listing), ["C13", "C14", "C15", "C16"]);
    }
}

#[test]
fn the_listing_is_capped_and_says_how_to_see_the_rest() {
    let registry = EnvironmentRegistry::new();
    for n in 1..=25 {
        registry.commit(qa_record(n, "", EnvironmentStatus::Running));
    }
    let (_, listing) = listed(registry.clone(), serde_json::json!({}));
    // The newest are kept: C6–C25.
    let expected: Vec<String> = (6..=25).map(|n| format!("C{n}")).collect();
    assert_eq!(expected.len(), MAX_LISTED_ENVIRONMENTS);
    assert_eq!(refs(&listing), expected);
    assert_eq!(listing["total"], 25);
    assert_eq!(listing["omitted"], 5);
    assert!(listing.get("hidden").is_none(), "{listing}");
    assert_eq!(
        listing["note"],
        "… 5 more (use `quecto container ls --all`)"
    );

    for n in 26..=27 {
        registry.commit(qa_record(n, "", EnvironmentStatus::Stopped));
    }
    registry.restore(vec![qa_record(30, "cli:other", EnvironmentStatus::Stopped)]);
    let (_, listing) = listed(registry.clone(), serde_json::json!({}));
    // This process's stopped C26–C27 are hidden with the foreign C30.
    assert_eq!(listing["hidden"], 3);
    assert_eq!(listing["omitted"], 5);
    assert_eq!(
        listing["note"],
        "3 not joinable (stopped, or other sessions'); \"all\":true lists them, also at most 20; \
         … 5 more (use `quecto container ls --all`)"
    );
    let (_, listing) = listed(registry, serde_json::json!({"all": true}));
    assert_eq!(listing["omitted"], 8);
    assert_eq!(
        listing["note"],
        "… 8 more (use `quecto container ls --all`)"
    );
}

#[test]
fn only_the_listed_metadata_keys_survive_and_only_as_clipped_text() {
    let registry = EnvironmentRegistry::new();
    let mut retained = qa_record(1, "", EnvironmentStatus::Retained);
    retained.metadata["retained"] = serde_json::json!("run r1 unfinished; kill_container ends it");
    retained.metadata["cause"] = serde_json::json!({"not": "text"});
    registry.commit(retained);
    let mut inspected = qa_record(2, "", EnvironmentStatus::Stopped);
    inspected.metadata["cause"] = serde_json::json!("oom-killed");
    inspected.metadata["inspect_status"] = serde_json::json!("dead");
    inspected.last_error = Some("é".repeat(MAX_TEXT_CHARS + 50));
    registry.commit(inspected);
    registry.commit(qa_record(3, "", EnvironmentStatus::Running));

    let (_, listing) = listed(registry, serde_json::json!({"all": true}));
    let rows = listing["containers"].as_array().unwrap();
    assert_eq!(
        rows[0]["metadata"],
        serde_json::json!({"retained": "run r1 unfinished; kill_container ends it"})
    );
    assert_eq!(
        rows[1]["metadata"],
        serde_json::json!({"cause": "oom-killed", "inspect_status": "dead"})
    );
    assert_eq!(
        rows[1]["last_error"],
        format!("{}…", "é".repeat(MAX_TEXT_CHARS))
    );
    assert!(rows[2].get("metadata").is_none(), "{}", rows[2]);
    assert!(rows[2].get("last_error").is_none(), "{}", rows[2]);
}

#[test]
fn free_text_is_redacted_before_it_is_clipped() {
    let registry = EnvironmentRegistry::new();
    let mut record = qa_record(1, "", EnvironmentStatus::CleanupFailed);
    record.last_error = Some(
        "kill failed: GET https://ci:hunter2@example.test/x?access_token=abc123 \
         and token=s3cr3t in the env"
            .into(),
    );
    record.metadata["retained"] = serde_json::json!("pull from https://u:p@host/r failed");
    record.metadata["cause"] = serde_json::json!("password: letmein");
    record.metadata["container"] = serde_json::json!("quecto-env-q000000001");
    registry.commit(record);
    let (_, listing) = listed(registry, serde_json::json!({}));
    let row = &listing["containers"][0];
    let text = row.to_string();
    for secret in ["hunter2", "abc123", "s3cr3t", "letmein", "u:p@"] {
        assert!(!text.contains(secret), "{secret} leaked: {row}");
    }
    assert_eq!(
        row["last_error"],
        "kill failed: GET https://***@example.test/x?[REDACTED] and [REDACTED] in the env"
    );
    assert_eq!(
        row["metadata"]["retained"],
        "pull from https://***@host/r failed"
    );
    assert_eq!(row["metadata"]["cause"], "[REDACTED]");
    // The container name is kept whole: logs are read by it.
    assert_eq!(row["metadata"]["container"], "quecto-env-q000000001");
    // Redacted first, then clipped: a secret straddling the cut is gone.
    let long = format!("{}token=abcdefgh", "x".repeat(MAX_TEXT_CHARS - 4));
    assert_eq!(
        safe_text(&long),
        format!("{}[RED…", "x".repeat(MAX_TEXT_CHARS - 4))
    );
}

#[test]
fn a_plain_container_name_passes_whole_and_anything_else_is_dropped() {
    for (name, kept) in [
        ("myproj-worker01", true),
        // Secret-shaped: dropped, a key after a `-` included.
        ("sk-abcdefghijkl", false),
        ("myproj-sk-abcdefghijkl", false),
        ("AKIAIOSFODNN7EXAMPLE", false),
        // A key prefix inside a word is no key (#2241): `task-runner01`
        // holds `sk-runner01` but is kept whole.
        ("myproj-task-runner01", true),
        ("quecto-env-AbCdEfGhIj", true),
        ("a", true),
        ("a.b_c-d", true),
        (&*"x".repeat(128), true),
        (&*"x".repeat(129), false),
        ("-leading-dash", false),
        ("", false),
        ("has space", false),
        ("x;rm -rf", false),
        ("https://u:p@host", false),
        ("naïve", false),
    ] {
        let registry = EnvironmentRegistry::new();
        let mut record = qa_record(1, "", EnvironmentStatus::Running);
        record.metadata["container"] = serde_json::json!(name);
        registry.commit(record);
        let (_, listing) = listed(registry, serde_json::json!({}));
        let row = &listing["containers"][0];
        match kept {
            true => assert_eq!(row["metadata"]["container"], name, "{row}"),
            false => assert!(row.get("metadata").is_none(), "{name:?}: {row}"),
        }
    }
    assert!(!is_container_name("sk-abcdefghijkl"));
    assert!(!is_container_name("AKIAIOSFODNN7EXAMPLE"));
    assert!(is_container_name("quecto-env-AbCdEfGhIj"));
    assert!(!is_container_name(&"x".repeat(129)));
}

#[test]
fn worst_case_fields_are_bounded_and_the_listing_fits_its_byte_budget() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let huge = "\u{1}".repeat(5_000);
    let mut restored = Vec::new();
    for n in 1..=20 {
        let mut record = qa_record(n, &format!("cli:{huge}"), EnvironmentStatus::Running);
        record.name = Some(huge.clone());
        record.script_name = huge.clone();
        record.repository = format!("https://token={huge}@example.test/{n}/{huge}");
        record.metadata["checkout"] = serde_json::json!(format!("/{huge}"));
        record.metadata["retained"] = serde_json::json!(huge.clone());
        record.metadata["cause"] = serde_json::json!(huge.clone());
        record.metadata["inspect_status"] = serde_json::json!(huge.clone());
        record.last_error = Some(format!("password={huge}"));
        restored.push(record);
    }
    registry.restore(restored);
    let (bytes, listing) = listed(registry, serde_json::json!({}));
    assert!(bytes <= MAX_LISTING_BYTES, "{bytes} bytes");
    let rows = listing["containers"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        assert!(row.get("metadata").is_none(), "metadata goes first: {row}");
        for key in ["name", "config", "session"] {
            let text = row[key].as_str().unwrap();
            assert!(text.chars().count() <= MAX_LABEL_CHARS + 1, "{key}");
        }
        for key in ["checkout", "last_error", "repository"] {
            let text = row[key].as_str().unwrap();
            assert!(text.chars().count() <= MAX_TEXT_CHARS + 1, "{key}");
        }
        assert!(!row["repository"].as_str().unwrap().contains("token="));
    }
    let omitted = listing["omitted"].as_u64().unwrap() as usize;
    assert_eq!(rows.len() + omitted, 20, "dropped rows are counted");
    let note = listing["note"].as_str().unwrap();
    assert!(
        note.contains("retained/cause/inspect_status left out to fit 16384 bytes"),
        "{note}"
    );
    assert!(note.contains(&format!("… {omitted} more")), "{note}");
}

#[test]
fn metadata_is_dropped_before_any_row_and_rows_go_in_keep_order() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let long = "r".repeat(MAX_TEXT_CHARS);
    for n in 1..=20 {
        let mut record = qa_record(n, CALLER, EnvironmentStatus::Running);
        record.metadata["retained"] = serde_json::json!(long.clone());
        record.metadata["cause"] = serde_json::json!(long.clone());
        registry.commit(record);
    }
    let (bytes, listing) = listed(registry, serde_json::json!({}));
    assert!(bytes <= MAX_LISTING_BYTES, "{bytes}");
    // Without metadata every row fits: none is dropped.
    assert_eq!(refs(&listing).len(), 20);
    assert!(listing.get("omitted").is_none(), "{listing}");
    assert_eq!(
        listing["note"],
        "retained/cause/inspect_status left out to fit 16384 bytes"
    );
    // Rows past the budget go newest-kept-first: the oldest is dropped.
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    for n in 1..=20 {
        let mut record = qa_record(n, CALLER, EnvironmentStatus::Running);
        record.last_error = Some("e".repeat(MAX_TEXT_CHARS));
        record.name = Some("n".repeat(MAX_LABEL_CHARS));
        record.script_name = "s".repeat(MAX_LABEL_CHARS);
        record.repository = format!("https://example.test/{n}/{}", "r".repeat(MAX_TEXT_CHARS));
        record.metadata["checkout"] = serde_json::json!(format!("/{}", "c".repeat(MAX_TEXT_CHARS)));
        registry.commit(record);
    }
    let (bytes, listing) = listed(registry, serde_json::json!({}));
    assert!(bytes <= MAX_LISTING_BYTES, "{bytes}");
    let shown = refs(&listing);
    assert!(shown.len() < 20, "{}", shown.len());
    assert_eq!(shown.last().map(String::as_str), Some("C20"));
    let first = 21 - shown.len();
    assert_eq!(shown[0], format!("C{first}"), "{shown:?}");
}

fn listing_of(rows: Vec<ListedEnvironment>, diagnostics: Vec<String>) -> EnvironmentListing {
    EnvironmentListing {
        total: rows.len(),
        shown: rows,
        hidden: 0,
        omitted: 0,
        diagnostics,
    }
}

fn row_of(
    record: crate::domain::environments::entities::environment_registry::EnvironmentRecord,
    keep_rank: usize,
) -> ListedEnvironment {
    ListedEnvironment {
        record,
        own: false,
        keep_rank,
    }
}

#[test]
fn one_worst_case_row_fits_the_real_budget_whole() {
    let ctl = "\u{1}".repeat(5_000);
    let mut record = qa_record(1, &ctl, EnvironmentStatus::CleanupFailed);
    record.environment_ref = "\u{1}".repeat(MAX_LABEL_CHARS);
    record.origin =
        crate::domain::environments::entities::environment_registry::EnvironmentOrigin::Restored;
    record.name = Some(ctl.clone());
    record.script_name = ctl.clone();
    record.repository = ctl.clone();
    record.metadata["checkout"] = serde_json::json!(format!("/{ctl}"));
    record.metadata["retained"] = serde_json::json!(ctl.clone());
    record.metadata["cause"] = serde_json::json!(ctl.clone());
    record.metadata["inspect_status"] = serde_json::json!(ctl.clone());
    record.metadata["container"] = serde_json::json!("x".repeat(128));
    record.last_error = Some(ctl.clone());
    let listing = listing_of(vec![row_of(record, 0)], vec![ctl.clone()]);
    let encoded = encode_listing(&listing);
    let bytes = encoded.to_string().len();
    assert!(bytes <= MAX_LISTING_BYTES, "{bytes} bytes");
    let row = &encoded["containers"][0];
    // Whole but for the free text: the container name and every field stay.
    for key in [
        "ref",
        "status",
        "config",
        "checkout",
        "name",
        "session",
        "repository",
        "last_error",
    ] {
        assert!(row.get(key).is_some(), "{key}: {row}");
    }
    assert_eq!(
        row["metadata"],
        serde_json::json!({"container": "x".repeat(128)})
    );
    assert!(encoded["diagnostics"][0].is_string());
    assert!(
        MAX_LISTING_BYTES - bytes > 1_000,
        "{bytes} bytes leaves too little margin"
    );
}

#[test]
fn a_row_that_cannot_fit_is_reduced_to_ref_and_status_not_a_panic() {
    let mut record = qa_record(1, "", EnvironmentStatus::Running);
    record.last_error = Some("e".repeat(MAX_TEXT_CHARS));
    let listing = listing_of(vec![row_of(record, 0)], vec![]);
    let encoded = encode_listing_within(&listing, 200);
    assert_eq!(
        encoded["containers"],
        serde_json::json!([{"ref": "C1", "status": "empty"}])
    );
    assert_eq!(
        encoded["note"],
        "rows reduced to ref and status to fit 200 bytes"
    );
}

#[test]
fn container_names_outlast_free_text_and_go_row_by_row_in_keep_order() {
    let rows: Vec<ListedEnvironment> = (1..=5)
        .map(|n| {
            let mut record = qa_record(n, CALLER, EnvironmentStatus::Running);
            record.metadata["retained"] = serde_json::json!("r".repeat(MAX_TEXT_CHARS));
            record.metadata["container"] = serde_json::json!(format!("quecto-env-q{n:09}"));
            // C5 is kept first, C1 dropped first.
            row_of(record, 5 - n as usize)
        })
        .collect();
    let listing = listing_of(rows, vec![]);
    let containers_kept = |encoded: &serde_json::Value| -> Vec<bool> {
        encoded["containers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["metadata"]["container"].is_string())
            .collect()
    };
    let whole = encode_listing_within(&listing, usize::MAX);
    assert!(whole["containers"][0]["metadata"]["retained"].is_string());
    // Shrink the budget byte by byte: the free text goes before any
    // container name, and the first container name to go is C1's — the
    // row a cap drops first — while every row stays.
    let mut free_text_gone_with_containers = false;
    let mut first_cut = None;
    for budget in (0..whole.to_string().len()).rev() {
        let encoded = encode_listing_within(&listing, budget);
        let kept = containers_kept(&encoded);
        let has_free_text = encoded["containers"][4]["metadata"]["retained"].is_string();
        match (has_free_text, kept.iter().all(|k| *k)) {
            (false, true) => free_text_gone_with_containers = true,
            (false, false) => {
                first_cut = Some(encoded);
                break;
            }
            (true, _) => assert!(kept.iter().all(|k| *k), "{encoded}"),
        }
    }
    assert!(free_text_gone_with_containers);
    let encoded = first_cut.expect("a small enough budget cuts a container");
    assert_eq!(containers_kept(&encoded), [false, true, true, true, true]);
    assert!(
        encoded["note"]
            .as_str()
            .unwrap()
            .contains("the container name of 1 row(s)"),
        "{encoded}"
    );
}

#[test]
fn a_foreign_row_with_no_session_key_names_no_session() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    registry.restore(vec![qa_record(1, "", EnvironmentStatus::Running)]);
    let (_, listing) = listed(registry, serde_json::json!({}));
    let row = &listing["containers"][0];
    assert_eq!(row["own"], false);
    assert!(row.get("session").is_none(), "{row}");
}

#[test]
fn the_checkout_is_secret_redacted_but_an_ordinary_path_is_untouched() {
    let registry = EnvironmentRegistry::new();
    let ordinary = "/home/qa/.quecto/containers/standard/state/env-q000000001/workspace/repo";
    let mut plain = qa_record(1, "", EnvironmentStatus::Running);
    plain.metadata["checkout"] = serde_json::json!(ordinary);
    registry.commit(plain);
    let mut leaky = qa_record(2, "", EnvironmentStatus::Running);
    leaky.metadata["checkout"] =
        serde_json::json!("/repo?token=ghp_abcdefghijklmnopqrstuvwxyz0123");
    registry.commit(leaky);
    let mut userinfo = qa_record(3, "", EnvironmentStatus::Running);
    userinfo.metadata["checkout"] = serde_json::json!("/srv/git://mirror@host/repo");
    registry.commit(userinfo);
    let (_, listing) = listed(registry, serde_json::json!({}));
    let rows = listing["containers"].as_array().unwrap();
    assert_eq!(rows[0]["checkout"], ordinary);
    assert_eq!(rows[1]["checkout"], "/repo?[REDACTED]");
    // No URL-userinfo rewriting: a path is not a URL.
    assert_eq!(rows[2]["checkout"], "/srv/git://mirror@host/repo");
}

#[test]
fn clip_keeps_text_up_to_the_limit_whole() {
    let exact = "x".repeat(MAX_TEXT_CHARS);
    assert_eq!(clip(&exact, MAX_TEXT_CHARS), exact);
    assert_eq!(clip("", MAX_TEXT_CHARS), "");
    assert_eq!(
        clip(&"x".repeat(MAX_TEXT_CHARS + 1), MAX_TEXT_CHARS),
        format!("{exact}…")
    );
}

#[test]
fn the_checkout_is_the_advertised_absolute_one_else_the_workspace() {
    let registry = EnvironmentRegistry::new();
    registry.commit(qa_record(1, "", EnvironmentStatus::Running));
    let mut relative = qa_record(2, "", EnvironmentStatus::Running);
    relative.metadata["checkout"] = serde_json::json!("workspace/repo");
    registry.commit(relative);
    let mut absent = qa_record(3, "", EnvironmentStatus::Running);
    absent.metadata = serde_json::json!({});
    registry.commit(absent);
    let (_, listing) = listed(registry, serde_json::json!({}));
    let checkouts: Vec<&str> = listing["containers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["checkout"].as_str().unwrap())
        .collect();
    let workspace =
        |n: u64| format!("/home/qa/.quecto/containers/standard/state/env-q{n:09}/workspace");
    assert_eq!(
        checkouts,
        [format!("{}/repo", workspace(1)), workspace(2), workspace(3)]
    );
}

#[test]
fn the_repository_is_named_once_only_when_every_row_shares_it_and_never_with_userinfo() {
    let registry = EnvironmentRegistry::new();
    let mut secret = qa_record(1, "", EnvironmentStatus::Running);
    secret.repository = "https://user:token@example.test/a.git".into();
    registry.commit(secret.clone());
    // One row: named on the row.
    let (_, listing) = listed(registry.clone(), serde_json::json!({}));
    assert!(listing.get("repository").is_none(), "{listing}");
    assert_eq!(
        listing["containers"][0]["repository"],
        "https://***@example.test/a.git"
    );
    // Two rows sharing it: named once, at the top.
    secret.environment_ref = "C2".into();
    registry.commit(secret);
    let (_, listing) = listed(registry.clone(), serde_json::json!({}));
    assert_eq!(listing["repository"], "https://***@example.test/a.git");
    assert!(listing["containers"][1].get("repository").is_none());
    // A third with another repository, and a sandbox with none: per row.
    registry.commit(qa_record(3, "", EnvironmentStatus::Running));
    let mut sandbox = qa_record(4, "", EnvironmentStatus::Running);
    sandbox.repository = String::new();
    registry.commit(sandbox);
    let (_, listing) = listed(registry, serde_json::json!({}));
    assert!(listing.get("repository").is_none(), "{listing}");
    let repositories: Vec<serde_json::Value> = listing["containers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["repository"].clone())
        .collect();
    assert_eq!(
        repositories,
        [
            serde_json::json!("https://***@example.test/a.git"),
            serde_json::json!("https://***@example.test/a.git"),
            serde_json::json!("https://github.com/platform-q-ai/quecto.git"),
            serde_json::Value::Null,
        ]
    );
}

#[test]
fn repositories_that_differ_only_in_userinfo_are_one_repository_to_the_model() {
    let registry = EnvironmentRegistry::new();
    for (n, userinfo) in [(1, "a:t1"), (2, "b:t2")] {
        let mut record = qa_record(n, "", EnvironmentStatus::Running);
        record.repository = format!("https://{userinfo}@example.test/r.git");
        registry.commit(record);
    }
    let (_, listing) = listed(registry, serde_json::json!({}));
    assert_eq!(listing["repository"], "https://***@example.test/r.git");
    for row in listing["containers"].as_array().unwrap() {
        assert!(row.get("repository").is_none(), "{row}");
    }
}

#[test]
fn sandboxes_that_share_no_repository_name_none() {
    let registry = EnvironmentRegistry::new();
    for n in 1..=2 {
        let mut sandbox = qa_record(n, "", EnvironmentStatus::Running);
        sandbox.repository = String::new();
        registry.commit(sandbox);
    }
    let (_, listing) = listed(registry, serde_json::json!({}));
    assert!(listing.get("repository").is_none(), "{listing}");
    for row in listing["containers"].as_array().unwrap() {
        assert!(row.get("repository").is_none(), "{row}");
    }
}

#[test]
fn a_named_foreign_environment_carries_its_name_and_session() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let mut named = qa_record(1, "cli:other", EnvironmentStatus::Running);
    named.name = Some("pr-env".into());
    named.created_at = None;
    registry.restore(vec![named]);
    let (_, listing) = listed(registry, serde_json::json!({}));
    let row = &listing["containers"][0];
    assert_eq!(row["name"], "pr-env");
    assert_eq!(row["session"], "cli:other");
    assert_eq!(row["own"], false);
    assert_eq!(row["restored"], true);
    assert!(row.get("created_at").is_none(), "{row}");
}
