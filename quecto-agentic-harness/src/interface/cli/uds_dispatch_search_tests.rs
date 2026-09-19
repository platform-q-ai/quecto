//! `search_session_metadata` through the real dispatch over the production
//! composition (#2010): the wire contract, hostile input on both sides, and
//! the row a selection is made from.
use super::super::fixture_tests::Fixture;
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_home::{AssociationProvenance, SessionHome, WorkspaceGroup};
use crate::domain::session_identity::SessionIdentity;
use crate::interface::cli::protocol::AgentCommand;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

async fn answer(fx: &mut Fixture, line: serde_json::Value) -> serde_json::Value {
    let id = line["id"].as_str().unwrap().to_string();
    let command: AgentCommand = serde_json::from_value(line).expect("a protocol command");
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    let mut ctx = fx.ctx();
    ctx.broadcast_tx = Some(tx);
    assert!(!super::super::dispatch_command(command, &mut ctx).await);
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(|frame| serde_json::from_str::<serde_json::Value>(&frame).unwrap())
        .find(|event| event["id"] == id.as_str())
        .expect("the correlated answer")
}

async fn search(fx: &mut Fixture, query: &str, scope: &str) -> serde_json::Value {
    let line = serde_json::json!({
        "type": "search_session_metadata", "id": "s1", "query": query, "scope": scope,
    });
    let event = answer(fx, line).await;
    assert_eq!(event["success"], true, "{event}");
    assert_eq!(event["command"], "search_session_metadata");
    event["data"].clone()
}

fn keys(data: &serde_json::Value) -> Vec<String> {
    let rows = data["sessions"].as_array().unwrap();
    rows.iter()
        .map(|row| row["key"].as_str().unwrap().to_string())
        .collect()
}

async fn saved(fx: &Fixture, key: &str, title: &str, home: Option<(PathBuf, Option<&str>)>) {
    let identity = SessionIdentity::from_persisted_key(key);
    if let Some((dir, repo)) = home {
        let group = match repo {
            Some(repo) => WorkspaceGroup::Git {
                common_dir: format!("/srv/{repo}/.git").into(),
            },
            None => WorkspaceGroup::Folder {
                directory: dir.clone(),
            },
        };
        let home = SessionHome {
            execution_dir: dir,
            group,
            provenance: AssociationProvenance::SavedHere,
        };
        fx.store.record_new_home(&identity, &home).unwrap();
    }
    let mut session = Session::new(identity.clone());
    session.messages.push(Message::user(title));
    fx.store.save(&session).await.unwrap();
    fx.store.release(&identity);
}

async fn seeded() -> Fixture {
    let fx = Fixture::new();
    saved(
        &fx,
        "cli:by-title",
        "Tune the ZEBRA cache",
        Some(("/w/one".into(), Some("r1"))),
    )
    .await;
    saved(
        &fx,
        "chat-1700000000-abc",
        "second",
        Some(("/w/two".into(), Some("r2"))),
    )
    .await;
    saved(
        &fx,
        "cli:by-repo",
        "third",
        Some(("/w/three".into(), Some("walrus"))),
    )
    .await;
    saved(
        &fx,
        "cli:by-path",
        "fourth",
        Some(("/w/heron-dir".into(), None)),
    )
    .await;
    saved(&fx, "cli:unscoped", "An old unscoped chat", None).await;
    fx
}

#[tokio::test]
async fn each_kind_of_metadata_finds_its_session_with_the_listing_row_and_what_matched() {
    let mut fx = seeded().await;
    let list = serde_json::json!({"type": "list_sessions", "id": "l1", "scope": "global"});
    let listed = answer(&mut fx, list).await["data"]["sessions"].clone();
    for (query, key, field, label) in [
        ("zebra", "cli:by-title", "title", Some("r1")),
        (
            "chat-1700000000-abc",
            "chat-1700000000-abc",
            "key",
            Some("r2"),
        ),
        ("WALRUS", "cli:by-repo", "repository", Some("walrus")),
        ("heron", "cli:by-path", "repository", Some("heron-dir")),
        ("w/heron", "cli:by-path", "path", Some("heron-dir")),
        ("unscoped", "cli:unscoped", "title", None),
    ] {
        let data = search(&mut fx, query, "global").await;
        assert_eq!(keys(&data), [key], "{query}: {data}");
        let row = &data["sessions"][0];
        assert_eq!(row["matched"][0], field, "{query}: {row}");
        assert_eq!(row["repositoryLabel"], serde_json::json!(label), "{query}");
        let same = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["key"] == key)
            .unwrap();
        for shared in [
            "title",
            "messageCount",
            "updatedUnixSecs",
            "homeState",
            "executionPath",
            "resumeEligible",
            "homeVersion",
        ] {
            assert_eq!(row[shared], same[shared], "{query}: {shared}");
        }
        assert_eq!(
            (&data["totalMatches"], &data["searched"]),
            (&serde_json::json!(1), &serde_json::json!(5))
        );
        assert_eq!(
            (&data["truncated"], &data["refused"]),
            (&serde_json::json!(false), &serde_json::Value::Null)
        );
    }
    let unscoped = search(&mut fx, "cli:unscoped", "global").await;
    let row = &unscoped["sessions"][0];
    assert_eq!(
        (&row["homeState"], &row["executionPath"]),
        (
            &serde_json::json!("legacy_unscoped"),
            &serde_json::Value::Null
        )
    );
    assert!(
        keys(&search(&mut fx, "zebra", "local").await).is_empty(),
        "local is this folder only"
    );
}

#[tokio::test]
async fn the_answer_echoes_generation_scope_limit_and_a_safe_query() {
    let mut fx = seeded().await;
    let line = serde_json::json!({
        "type": "search_session_metadata", "id": "g1", "query": "a\u{1b}[31m\u{202e}b",
        "scope": "global", "generation": 41, "limit": 2,
    });
    let data = answer(&mut fx, line).await["data"].clone();
    assert_eq!(
        (&data["generation"], &data["scope"], &data["limit"]),
        (
            &serde_json::json!(41),
            &serde_json::json!("global"),
            &serde_json::json!(2)
        )
    );
    assert_eq!(data["query"], "a\u{fffd}[31m\u{fffd}b");
    let defaults = serde_json::json!({"type": "search_session_metadata", "id": "g2", "query": ""});
    let data = answer(&mut fx, defaults).await["data"].clone();
    assert_eq!(
        (&data["generation"], &data["scope"], &data["limit"]),
        (
            &serde_json::json!(0),
            &serde_json::json!("local"),
            &serde_json::json!(200)
        )
    );
    let capped = serde_json::json!({"type": "search_session_metadata", "id": "g3", "query": "", "scope": "global", "limit": 2});
    let data = answer(&mut fx, capped).await["data"].clone();
    assert_eq!(
        (keys(&data).len(), &data["totalMatches"], &data["truncated"]),
        (2, &serde_json::json!(5), &serde_json::json!(true))
    );
}

#[tokio::test]
async fn a_malformed_search_is_rejected_at_the_protocol_boundary() {
    for line in [
        serde_json::json!({"type": "search_session_metadata", "id": "p"}),
        serde_json::json!({"type": "search_session_metadata", "id": "p", "query": 7}),
        serde_json::json!({"type": "search_session_metadata", "id": "p", "query": "x", "scope": "everywhere"}),
        serde_json::json!({"type": "search_session_metadata", "id": "p", "query": "x", "generation": -1}),
        serde_json::json!({"type": "search_session_metadata", "id": "p", "query": "x", "generation": "3"}),
        serde_json::json!({"type": "search_session_metadata", "id": "p", "query": "x", "limit": 1.5}),
    ] {
        assert!(
            serde_json::from_value::<AgentCommand>(line.clone()).is_err(),
            "{line}"
        );
    }
    let command: AgentCommand = serde_json::from_value(
        serde_json::json!({"type": "search_session_metadata", "id": "ok", "query": "x"}),
    )
    .unwrap();
    assert_eq!(
        (command.id(), command.type_name()),
        (Some("ok"), "search_session_metadata")
    );
}

#[tokio::test]
async fn hostile_queries_are_literal_bounded_and_answered_the_same_way_twice() {
    let mut fx = seeded().await;
    saved(
        &fx,
        "cli:meta",
        "why does a.*b [x]+ (y|z) \\d$ 100%_ fail?",
        None,
    )
    .await;
    for query in [
        ".*",
        "a.*b",
        "[x]+",
        "(y|z)",
        "\\d$",
        "%_",
        "*",
        "?",
        "**/*",
        "$(rm -rf /)",
        "`id`",
        "'; DROP TABLE sessions;--",
    ] {
        let first = search(&mut fx, query, "global").await;
        let matches_literal = "why does a.*b [x]+ (y|z) \\d$ 100%_ fail?".contains(query);
        assert_eq!(
            keys(&first),
            if matches_literal {
                vec!["cli:meta".to_string()]
            } else {
                vec![]
            },
            "{query}"
        );
        assert_eq!(
            first,
            search(&mut fx, query, "global").await,
            "{query}: deterministic"
        );
    }
    // Nothing visible: every session, not an error.
    let invisible = search(&mut fx, "\u{200b}\u{202e}\u{0}\u{7}", "global").await;
    assert_eq!(invisible["totalMatches"], 6);
    assert_eq!(invisible["query"], "\u{fffd}\u{fffd}\u{fffd}\u{fffd}");
    // Over-long: refused whole, the echo bounded, nothing searched.
    let long = search(&mut fx, &"z".repeat(100_000), "global").await;
    assert_eq!(
        long["refused"],
        "query too long: 100000 characters (at most 256 are searched)"
    );
    assert_eq!(
        (
            &long["searched"],
            keys(&long).len(),
            long["query"].as_str().unwrap().len()
        ),
        (&serde_json::json!(0), 0, 256)
    );
}

#[tokio::test]
async fn hostile_metadata_is_searchable_and_never_reaches_the_wire_raw() {
    let mut fx = Fixture::new();
    let odd = PathBuf::from(std::ffi::OsStr::from_bytes(
        b"/w/caf\xe9/pro\xe2\x80\xaeject",
    ));
    saved(
        &fx,
        "cli:odd",
        "de\u{202e}pl\u{200b}oy \u{1b}]0;owned\u{7} now",
        Some((odd, None)),
    )
    .await;
    for (query, field) in [
        ("deploy", "title"),
        ("caf", "path"),
        ("project", "repository"),
    ] {
        let data = search(&mut fx, query, "global").await;
        assert_eq!(keys(&data), ["cli:odd"], "{query}");
        assert_eq!(data["sessions"][0]["matched"][0], field);
        let wire = serde_json::to_string(&data).unwrap();
        let raw: String = serde_json::from_str::<serde_json::Value>(&wire)
            .unwrap()
            .to_string();
        for hidden in ['\u{1b}', '\u{7}', '\u{202e}', '\u{200b}'] {
            let shown = data["sessions"][0].to_string();
            assert!(
                !shown.contains(hidden) && !raw.contains(&format!("\\u{:04x}", hidden as u32)),
                "{query}: {hidden:?}"
            );
        }
        assert!(
            data["sessions"][0]["executionPath"]
                .as_str()
                .unwrap()
                .contains("caf\u{fffd}")
        );
    }
}

#[tokio::test]
async fn a_searched_rows_version_restores_it_and_a_stale_one_is_refused() {
    let mut fx = Fixture::new();
    crate::interface::cli::uds::dispatch_session_roster_tests::seed_home(&fx.store, "cli:picked")
        .await;
    saved(&fx, "cli:picked", "the PICKED one", None).await;
    let data = search(&mut fx, "picked", "local").await;
    let row = data["sessions"][0].clone();
    assert_eq!(
        (&row["key"], &row["resumeEligible"]),
        (&serde_json::json!("cli:picked"), &serde_json::json!(true))
    );
    let stale = serde_json::json!({
        "type": "resume_session", "id": "r0", "session": "cli:picked",
        "expectedHomeVersion": "h1-0000000000000000",
    });
    assert_eq!(
        answer(&mut fx, stale).await["data"]["code"],
        "stale_home_version"
    );
    let pick = serde_json::json!({
        "type": "resume_session", "id": "r1", "session": row["key"],
        "expectedHomeVersion": row["homeVersion"],
    });
    let resumed = answer(&mut fx, pick).await;
    assert_eq!(resumed["success"], true, "{resumed}");
    assert_eq!(fx.current_session_key(), "cli:picked");
}
