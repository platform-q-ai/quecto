//! Contract of the metadata query (#2010) over the REAL adapters: the file
//! session store, its home authority and derived index, real Git/filesystem
//! discovery. The query sees exactly the sessions a listing sees, with the
//! version token a listing carries; it is answered from stamps and the index
//! (a counting store and a counting catalogue prove no transcript is read to
//! match); a corrupt index or record is recovered or diagnosed without hiding
//! a sibling; and a selection made from an answer still passes — or fails —
//! the resume transaction's own checks.
use super::resume_fixture::{World, identity};
use quecto::application::sessions::dto::{
    ListedSession, ResumeOutcome, ResumeRequest, ResumeSavedSessionError,
    SearchSessionMetadataRequest, SearchSessionMetadataResult, SessionListScope,
};
use quecto::application::sessions::ports::SessionStore;
use quecto::application::sessions::ports::session_home::SessionHomeCatalogue;
use quecto::application::sessions::use_cases::SearchSessionMetadata;
use quecto::composition::session_home::session_home_in;
use quecto::domain::message::Message;
use quecto::domain::resume_decision::HomeVersion;
use quecto::domain::session::Session;
use quecto::domain::session_home::SessionHomeScope;
use quecto::domain::session_metadata_search::MatchedField;
use quecto::infrastructure::persistence::session_home_catalogue::FileSessionHomeCatalogue;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

async fn search(
    world: &World,
    query: &str,
    scope: SessionListScope,
) -> SearchSessionMetadataResult {
    let request = SearchSessionMetadataRequest {
        query: query.into(),
        scope,
        ..Default::default()
    };
    let answer = world.handles.discovery.search(&request).await;
    answer.expect("metadata search")
}

fn keys(result: &SearchSessionMetadataResult) -> Vec<&str> {
    result
        .rows
        .iter()
        .map(|row| row.session.summary.key.as_str())
        .collect()
}

async fn listed(world: &World) -> Vec<ListedSession> {
    let rows = world.handles.discovery.list(SessionListScope::Global);
    rows.await.expect("global listing").sessions
}

/// Scoped here and elsewhere, legacy, an uninterpretable sidecar and a record
/// the strict catalogue rejects (an append cut short by a crash).
async fn world_of_every_record_state() -> World {
    let world = World::new().await;
    world.saved_in("cli:mine", &world.here).await;
    world
        .saved_in("cli:theirs", &world.base.join("../elsewhere"))
        .await;
    world.save_transcript("cli:legacy").await;
    world
        .saved_in("cli:broken", &world.base.join("../broken"))
        .await;
    std::fs::write(world.layout.home_file(&identity("cli:broken")), b"{broken").unwrap();
    let crashed = world
        .saved_in("cli:crashed", &world.base.join("../crashed"))
        .await;
    assert!(crashed.exists());
    let record = world.layout.session_file(&identity("cli:crashed"));
    let mut bytes = std::fs::read(&record).unwrap();
    bytes.extend_from_slice(b"\n{\"type\":\"append\",\"messages\":[");
    std::fs::write(&record, bytes).unwrap();
    world
}

#[tokio::test]
async fn the_metadata_query_sees_exactly_the_sessions_and_homes_a_listing_sees() {
    let world = world_of_every_record_state().await;
    for pass in ["cold index", "index-seeded"] {
        let listing = listed(&world).await;
        let catalogue = FileSessionHomeCatalogue::with_store(world.store.clone());
        let snapshot = catalogue.metadata().await.expect("metadata");
        let mut queried: Vec<_> = snapshot
            .records
            .iter()
            .map(|r| (r.summary.clone(), r.home.clone()))
            .collect();
        let mut shown: Vec<_> = listing
            .iter()
            .map(|row| (row.summary.clone(), row.home.clone()))
            .collect();
        queried.sort_by(|a, b| a.0.key.cmp(&b.0.key));
        shown.sort_by(|a, b| a.0.key.cmp(&b.0.key));
        assert_eq!(queried, shown, "{pass}");
        assert!(
            queried
                .iter()
                .any(|(summary, _)| summary.key == "cli:crashed"),
            "{pass}: {queried:?}"
        );
        assert!(
            snapshot
                .records
                .windows(2)
                .all(|w| w[0].summary.updated_unix_secs >= w[1].summary.updated_unix_secs),
            "{pass}: newest first"
        );
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|d| d.starts_with("cli_crashed.json: session record unavailable")),
            "{pass}: {:?}",
            snapshot.diagnostics
        );
        // R1-H4: a sidecar that needs repair is named, as a record is.
        let sidecar = world.layout.home_file(&identity("cli:broken"));
        let sidecar = sidecar.file_name().unwrap().to_string_lossy().into_owned();
        let repairs = snapshot.diagnostics.iter();
        let repairs: Vec<_> = repairs
            .filter(|d| d.contains("home needs repair"))
            .collect();
        assert_eq!(repairs.len(), 1, "{pass}: {repairs:?}");
        assert!(
            repairs[0].starts_with(&format!("{sidecar}: home needs repair: ")),
            "{pass}: {repairs:?}"
        );
    }
}

#[tokio::test]
async fn a_search_rows_version_is_the_listed_and_the_authoritative_version() {
    let world = world_of_every_record_state().await;
    let authority = FileSessionHomeCatalogue::with_store(world.store.clone());
    for pass in ["cold index", "index-seeded"] {
        let listing = listed(&world).await;
        let found = search(&world, "history of", SessionListScope::Global).await;
        assert_eq!(
            found.rows.len(),
            listing.len(),
            "{pass}: every title matches"
        );
        let mut states = std::collections::BTreeSet::new();
        for row in &found.rows {
            let key = row.session.summary.key.as_str();
            let scope = authority.read(&identity(key)).unwrap();
            assert_eq!(
                row.session.home_version(),
                HomeVersion::of(&identity(key), &scope),
                "{pass}: {key}"
            );
            let same = listing
                .iter()
                .find(|l| l.summary.key == key)
                .expect("listed too");
            assert_eq!(
                row.session.home_version(),
                same.home_version(),
                "{pass}: {key}"
            );
            assert_eq!(
                row.session.resume_eligible, same.resume_eligible,
                "{pass}: {key}"
            );
            states.insert(match scope {
                SessionHomeScope::Scoped(_) => "scoped",
                SessionHomeScope::LegacyUnscoped => "legacy",
                SessionHomeScope::Unavailable(_) => "unavailable",
            });
        }
        assert_eq!(
            states.len(),
            3,
            "{pass}: scoped, legacy and unavailable are all exercised"
        );
    }
}

#[tokio::test]
async fn a_selection_made_from_an_answer_still_answers_to_the_resume_transaction() {
    let world = world_of_every_record_state().await;
    world.saved_in("cli:moved", &world.here).await;
    world.saved_in("cli:deleted", &world.here).await;
    let found = search(&world, "cli:mine", SessionListScope::Local).await;
    assert_eq!(
        (keys(&found), &found.rows[0].matched[..]),
        (
            vec!["cli:mine"],
            &[MatchedField::Key, MatchedField::Title][..]
        )
    );
    let select = |result: &SearchSessionMetadataResult| ResumeRequest {
        target: result.rows[0].session.summary.key.clone(),
        expected_home_version: Some(result.rows[0].session.home_version()),
    };
    // Re-homed between the answer and the selection: the token is stale.
    let moved = search(&world, "cli:moved", SessionListScope::Global).await;
    let elsewhere = world
        .saved_in("cli:elsewhere", &world.base.join("../elsewhere"))
        .await;
    assert!(elsewhere.exists());
    std::fs::copy(
        world.layout.home_file(&identity("cli:elsewhere")),
        world.layout.home_file(&identity("cli:moved")),
    )
    .unwrap();
    let (result, _) = world.request(&select(&moved)).await;
    assert!(
        matches!(result, Err(ResumeSavedSessionError::StaleHomeVersion)),
        "{result:?}"
    );
    // Deleted between the answer and the selection: nothing is activated.
    let deleted = search(&world, "cli:deleted", SessionListScope::Global).await;
    std::fs::remove_file(world.layout.session_file(&identity("cli:deleted"))).unwrap();
    let (result, _) = world.request(&select(&deleted)).await;
    assert!(
        matches!(&result, Err(ResumeSavedSessionError::NotFound(name)) if name == "cli:deleted"),
        "{result:?}"
    );
    // Untouched: the searched row restores.
    let (result, _) = world.request(&select(&found)).await;
    assert!(
        matches!(result, Ok(ResumeOutcome::Resumed(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn only_metadata_is_matched_never_transcript_content() {
    let world = World::new().await;
    let mut session = Session::new(identity("cli:deep"));
    session.messages.push(Message::user("a plain title"));
    session.messages.push(Message::assistant(
        "the answer mentions XYLOPHONE-7",
        Vec::new(),
    ));
    session
        .messages
        .push(Message::user("and so does XYLOPHONE-7 the follow-up"));
    world.store.save(&session).await.unwrap();
    world.store.release(&identity("cli:deep"));
    assert!(keys(&search(&world, "xylophone-7", SessionListScope::Global).await).is_empty());
    assert_eq!(
        keys(&search(&world, "plain title", SessionListScope::Global).await),
        ["cli:deep"]
    );
    // The derived index the query is answered from never held the content.
    let _ = search(&world, "xylophone-7", SessionListScope::Global).await;
    let index = std::fs::read_to_string(world.layout.home_catalogue_file()).unwrap();
    assert!(!index.contains("XYLOPHONE-7") && index.contains("a plain title"));
}

#[tokio::test]
async fn a_corrupt_index_and_a_corrupt_record_are_recovered_or_named_without_hiding_a_match() {
    let world = world_of_every_record_state().await;
    let _ = search(&world, "", SessionListScope::Global).await;
    std::fs::write(world.layout.home_catalogue_file(), b"\x00garbage").unwrap();
    std::fs::write(
        world.base.join("sessions/cli_rotten.json"),
        b"{\"key\":\"cli:rotten\",",
    )
    .unwrap();
    let found = search(&world, "theirs", SessionListScope::Global).await;
    assert_eq!(keys(&found), ["cli:theirs"]);
    assert!(found.freshness.rebuilt, "{:?}", found.freshness);
    let diagnostics = &found.freshness.diagnostics;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("rebuilt from authority")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.starts_with("cli_rotten.json: ")),
        "{diagnostics:?}"
    );
    let again = search(&world, "theirs", SessionListScope::Global).await;
    assert!(!again.freshness.rebuilt && keys(&again) == ["cli:theirs"]);
    // A stale index (authority moved on since it was written) is refreshed.
    world.saved_in("cli:newer", &world.here).await;
    assert_eq!(
        keys(&search(&world, "cli:newer", SessionListScope::Local).await),
        ["cli:newer"]
    );
    // Exact-key resolution never needed the index.
    std::fs::write(world.layout.home_catalogue_file(), b"\x00garbage").unwrap();
    let decision = world.decision("cli:theirs").await;
    assert_eq!(
        decision.kind,
        quecto::domain::resume_decision::ResumeDecisionKind::CrossFolder
    );
}

/// The scaled fixture of the epic's realistic-store criterion: on a warm
/// index a search — any number of them, in this process or a new one — reads
/// no transcript, neither through the catalogue nor through the store's
/// summary walk.
#[cfg(feature = "test-support")]
#[tokio::test]
async fn two_thousand_records_are_searched_without_reading_a_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let here = dir.path().join("here");
    std::fs::create_dir_all(&here).unwrap();
    let here = here.canonicalize().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let count = 2_000;
    for n in 0..count {
        let id = identity(&format!("chat-scale-{n:04}"));
        let mut session = Session::new(id.clone());
        session
            .messages
            .push(Message::user(format!("scaled title number-{n:04}")));
        session.messages.push(Message::assistant(
            format!("CONTENT-ONLY-{n:04} ").repeat(40),
            Vec::new(),
        ));
        store.save(&session).await.unwrap();
        store.release(&id);
    }
    // R1-H1: two records that can never be summarised strictly — one neither
    // half can parse (an unterminated snapshot), one large append cut short
    // (the walk lists it, the strict catalogue rejects it). A failure is
    // cached by stamp exactly as a success is.
    let rot = layout.session_file(&identity("chat-rot"));
    std::fs::write(
        &rot,
        br#"{"key":"chat-rot","messages":[{"role":"user","content":"AAAA"#,
    )
    .unwrap();
    let cut_id = identity("chat-cut");
    let mut cut = Session::new(cut_id.clone());
    cut.messages.push(Message::user("cut short title"));
    store.save(&cut).await.unwrap();
    store.release(&cut_id);
    let cut = layout.session_file(&cut_id);
    let mut bytes = std::fs::read(&cut).unwrap();
    bytes.extend_from_slice(
        b"\n{\"type\":\"append\",\"messages\":[{\"role\":\"user\",\"content\":\"",
    );
    bytes.extend(std::iter::repeat_n(b'A', 2 << 20));
    std::fs::write(&cut, bytes).unwrap();
    let named_once = |found: &SearchSessionMetadataResult, pass: &str| {
        for file in [&rot, &cut] {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let lines = found.freshness.diagnostics.iter();
            let named = lines.filter(|d| d.starts_with(&name)).count();
            assert_eq!(
                named, 1,
                "{pass}: {name}: {:?}",
                found.freshness.diagnostics
            );
        }
    };
    let catalogue = Arc::new(FileSessionHomeCatalogue::with_store(store.clone()));
    let mut home = session_home_in(store.clone(), Ok(here.clone()));
    home.catalogue = catalogue.clone();
    let warm = SearchSessionMetadata::new(home);
    let ask = |query: &str| SearchSessionMetadataRequest {
        query: query.into(),
        scope: SessionListScope::Global,
        ..Default::default()
    };
    let first = warm.search(&ask("number-0042")).await.unwrap();
    assert_eq!((first.rows.len(), first.searched), (1, count + 1));
    named_once(&first, "first");
    let reads = (
        catalogue.transcript_reads(),
        store.summary_transcript_reads(),
    );
    assert_eq!(
        reads,
        (count + 2, count + 2),
        "a fresh index validates each record once, the two bad ones included"
    );
    let started = std::time::Instant::now();
    // `expected` literal rows come first; a term found nowhere literally may
    // still match a title as a subsequence (#2043: `number-19` ⊂
    // "number-0192"), ranked below every literal row and counted.
    for (query, expected) in [
        ("number-19", 100),
        ("chat-scale-0007", 1),
        ("content-only", 0),
        ("scaled", count),
    ] {
        let found = warm.search(&ask(query)).await.unwrap();
        let literal = found
            .rows
            .iter()
            .take_while(|row| !row.matched.contains(&MatchedField::TitleFuzzy))
            .count();
        assert_eq!(literal, expected.min(found.rows.len()), "{query}");
        assert!(found.total_matches >= expected, "{query}");
        named_once(&found, query);
    }
    let elapsed = started.elapsed();
    assert_eq!(
        (
            catalogue.transcript_reads(),
            store.summary_transcript_reads()
        ),
        reads,
        "warm: zero reads"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "four searches took {elapsed:?}"
    );
    // A new process over the same directory: seeded from the index alone.
    let cold_store = Arc::new(FileSessionStore::new(layout));
    let cold_catalogue = Arc::new(FileSessionHomeCatalogue::with_store(cold_store.clone()));
    let mut home = session_home_in(cold_store.clone(), Ok(here));
    home.catalogue = cold_catalogue.clone();
    let cold = SearchSessionMetadata::new(home);
    let found = cold.search(&ask("number-1999")).await.unwrap();
    assert_eq!(found.rows.len(), 1);
    named_once(&found, "cold");
    let cut_row = cold.search(&ask("cut short")).await.unwrap();
    assert_eq!(
        keys(&cut_row),
        ["chat-cut"],
        "a rejected record stays findable"
    );
    assert_eq!(found.rows[0].session.home, SessionHomeScope::LegacyUnscoped);
    assert_eq!(
        (
            cold_catalogue.transcript_reads(),
            cold_store.summary_transcript_reads()
        ),
        (2, 2),
        "cold: the two rejected records once per half (R2-H2), nothing else, and not again"
    );
}
