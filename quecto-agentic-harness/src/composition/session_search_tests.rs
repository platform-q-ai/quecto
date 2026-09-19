use crate::application::sessions::dto::{
    SaveTrigger, SearchSessionMetadataRequest, SessionListScope,
};
use crate::composition::sessions::{SessionLoopInputs, build_session_handles};
use crate::domain::message::Message;
use crate::domain::session_home::SessionHomeScope;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::session_metadata_search::MatchedField;

fn inputs(base: &std::path::Path, identity: SessionIdentity) -> SessionLoopInputs {
    SessionLoopInputs {
        base_dir: base.to_path_buf(),
        identity,
        ephemeral: false,
        system_prompt: String::new(),
        spill_store: None,
        durable_prefix: crate::application::durable_prefix::DurablePrefixLatch::shared(),
        workflow_state: None,
        subagent_registry: None,
    }
}

/// Composition smoke: the production graph saves a session with its home, and
/// a restarted graph finds it by title, key and path — the row a listing
/// shows, version and eligibility included — and by nothing else.
#[tokio::test]
async fn the_production_graph_searches_what_it_saved_by_title_key_and_path() {
    let base = tempfile::tempdir().unwrap();
    let identity = SessionIdentity::named_cli("composed-search").unwrap();
    let handles = build_session_handles(inputs(base.path(), identity.clone()));
    handles.switch.resume.open_at_startup().await.unwrap();
    let mut messages = vec![Message::user("COMPOSED needle title")];
    let saved = handles
        .save_session
        .save(&mut messages, SaveTrigger::OrdinaryExit);
    saved.await.unwrap();
    handles.store.release(&identity);
    drop(handles);

    let restarted = build_session_handles(inputs(base.path(), identity.clone()));
    let listed = restarted
        .discovery
        .list(SessionListScope::Local)
        .await
        .unwrap();
    let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let folder = here.file_name().unwrap().to_string_lossy().into_owned();
    for (query, field) in [
        ("needle", MatchedField::Title),
        ("cli:composed-search", MatchedField::Key),
        (folder.as_str(), MatchedField::Path),
    ] {
        let request = SearchSessionMetadataRequest {
            query: query.into(),
            ..Default::default()
        };
        let found = restarted.discovery.search(&request).await.unwrap();
        assert_eq!(found.rows.len(), 1, "{query}");
        let row = &found.rows[0];
        assert!(row.matched.contains(&field), "{query}: {:?}", row.matched);
        assert!(row.session.resume_eligible);
        assert_eq!(
            row.session.home_version(),
            listed.sessions[0].home_version()
        );
        let SessionHomeScope::Scoped(home) = &row.session.home else {
            panic!("a new save has an authoritative home")
        };
        assert_eq!(home.execution_dir, here);
    }
    let absent = SearchSessionMetadataRequest {
        query: "no-such-metadata-anywhere".into(),
        scope: SessionListScope::Global,
        ..Default::default()
    };
    assert!(
        restarted
            .discovery
            .search(&absent)
            .await
            .unwrap()
            .rows
            .is_empty()
    );
}
