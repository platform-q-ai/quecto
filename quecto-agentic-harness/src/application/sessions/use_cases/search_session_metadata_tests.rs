use super::*;
use crate::application::sessions::dto::{QueryGeneration, SearchLimit};
use crate::application::sessions::ports::session_home::{
    HomeCatalogueSnapshot, SessionHomeCatalogue, SessionMetadataSnapshot, WorkspaceDiscovery,
};
use crate::domain::resume_decision::HomeVersion;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::{AssociationProvenance, WorkspaceGroup};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::session_metadata_search::QueryRefusal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const HERE: &str = "/work/quecto/src";

fn git(dir: &str, repo: &str) -> SessionHomeScope {
    SessionHomeScope::Scoped(SessionHome {
        execution_dir: dir.into(),
        group: WorkspaceGroup::Git {
            common_dir: format!("/work/{repo}/.git").into(),
        },
        provenance: AssociationProvenance::SavedHere,
    })
}

fn record(
    key: &str,
    title: &str,
    updated: Option<u64>,
    home: SessionHomeScope,
) -> SessionMetadataRecord {
    SessionMetadataRecord {
        summary: SessionSummary {
            key: key.into(),
            identity: SessionIdentity::from_persisted_key(key),
            title: title.into(),
            message_count: 2,
            updated_unix_secs: updated,
        },
        home,
    }
}

/// A catalogue that answers only the metadata query: any other call is a
/// search that read exact authority or mutated it.
struct Metadata {
    answer: Mutex<Option<Result<SessionMetadataSnapshot, DomainError>>>,
    queries: Mutex<usize>,
}
impl Metadata {
    fn of(records: Vec<SessionMetadataRecord>) -> Arc<Self> {
        Self::answering(Ok(SessionMetadataSnapshot {
            records,
            diagnostics: vec!["cli_bad.json: session record unavailable".into()],
            rebuilt: true,
        }))
    }
    fn answering(answer: Result<SessionMetadataSnapshot, DomainError>) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Some(answer)),
            queries: Mutex::new(0),
        })
    }
}
impl SessionHomeCatalogue for Metadata {
    fn read(&self, _: &SessionIdentity) -> Result<SessionHomeScope, DomainError> {
        panic!("search never reads exact authority")
    }
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError> {
        panic!("search asks the metadata query, not the home listing")
    }
    fn record_new(&self, _: &SessionIdentity, _: &SessionHome) -> Result<(), DomainError> {
        panic!("search never mutates authority")
    }
    fn discard_orphan(&self, _: &SessionIdentity) -> Result<(), DomainError> {
        panic!("search never mutates authority")
    }
    fn metadata(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SessionMetadataSnapshot, DomainError>> + Send + '_>>
    {
        *self.queries.lock().unwrap() += 1;
        let answer = self.answer.lock().unwrap().take().expect("one query");
        Box::pin(async move { answer })
    }
}
use std::future::Future;
use std::pin::Pin;

struct Discovery(Mutex<Vec<PathBuf>>);
impl WorkspaceDiscovery for Discovery {
    fn discover(&self, path: &Path) -> Result<SessionHome, DomainError> {
        self.0.lock().unwrap().push(path.into());
        match git(&path.to_string_lossy(), "quecto") {
            SessionHomeScope::Scoped(home) if path != Path::new("/nowhere") => Ok(home),
            _ => Err(DomainError::Session("not discoverable".into())),
        }
    }
}

struct Rig {
    search: SearchSessionMetadata,
    catalogue: Arc<Metadata>,
    discovery: Arc<Discovery>,
}
fn rig_at(dir: &str, catalogue: Arc<Metadata>) -> Rig {
    let discovery = Arc::new(Discovery(Mutex::new(Vec::new())));
    let home = SessionHomeContext::at(catalogue.clone(), discovery.clone(), dir.into());
    Rig {
        search: SearchSessionMetadata::new(home),
        catalogue,
        discovery,
    }
}

fn seeded() -> Vec<SessionMetadataRecord> {
    vec![
        record(
            "chat-1-title",
            "Tune the ZEBRA cache",
            Some(40),
            git("/work/alpha/a", "alpha"),
        ),
        record(
            "chat-2-key",
            "second",
            Some(30),
            git("/work/beta/b", "beta"),
        ),
        record(
            "chat-3-repo",
            "third",
            Some(20),
            git("/elsewhere/checkout", "walrus"),
        ),
        record(
            "chat-4-path",
            "fourth",
            Some(10),
            git("/work/gamma/heron-dir", "gamma"),
        ),
        record(
            "cli:legacy",
            "An unscoped chat",
            Some(50),
            SessionHomeScope::LegacyUnscoped,
        ),
        record("cli:mine", "local work", Some(5), git(HERE, "quecto")),
    ]
}

async fn keyed(rig: &Rig, query: &str, scope: SessionListScope) -> SearchSessionMetadataResult {
    let request = SearchSessionMetadataRequest {
        query: query.into(),
        scope,
        ..Default::default()
    };
    rig.search.search(&request).await.unwrap()
}

async fn keys(rig: &Rig, query: &str, scope: SessionListScope) -> Vec<String> {
    let result = keyed(rig, query, scope).await;
    result
        .rows
        .iter()
        .map(|row| row.session.summary.key.clone())
        .collect()
}

#[tokio::test]
async fn title_key_repository_and_path_each_find_exactly_their_session() {
    for (query, key, field) in [
        ("zebra", "chat-1-title", MatchedField::Title),
        ("chat-2-key", "chat-2-key", MatchedField::Key),
        ("walrus", "chat-3-repo", MatchedField::Repository),
        ("heron-dir", "chat-4-path", MatchedField::Path),
    ] {
        let rig = rig_at(HERE, Metadata::of(seeded()));
        let request = SearchSessionMetadataRequest {
            query: query.into(),
            scope: SessionListScope::Global,
            generation: QueryGeneration(7),
            limit: SearchLimit::default(),
        };
        let result = rig.search.search(&request).await.unwrap();
        assert_eq!(result.rows.len(), 1, "{query}");
        let row = &result.rows[0];
        assert_eq!(row.session.summary.key, key);
        assert_eq!(row.matched, [field]);
        assert_eq!(
            (result.total_matches, result.searched),
            (1, 6),
            "unscoped searched, not matched"
        );
        assert_eq!(result.generation, QueryGeneration(7));
        assert_eq!(result.scope, SessionListScope::Global);
        assert!(!result.truncated() && result.refused.is_none());
        assert_eq!(
            row.session.home_version(),
            HomeVersion::of(&row.session.summary.identity, &row.session.home),
            "a search row carries the listing's version token"
        );
    }
}

#[tokio::test]
async fn an_unscoped_session_is_found_by_title_and_key_and_labelled_by_its_home_state() {
    let rig = rig_at(HERE, Metadata::of(seeded()));
    let request = SearchSessionMetadataRequest {
        query: "unscoped".into(),
        scope: SessionListScope::Global,
        ..Default::default()
    };
    let result = rig.search.search(&request).await.unwrap();
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert_eq!(row.session.home, SessionHomeScope::LegacyUnscoped);
    assert_eq!(
        (row.repository_label.clone(), row.session.resume_eligible),
        (None, false)
    );
    let rig = rig_at(HERE, Metadata::of(seeded()));
    assert_eq!(
        keys(&rig, "cli:legacy", SessionListScope::Global).await,
        ["cli:legacy"]
    );
}

#[tokio::test]
async fn local_scope_searches_only_this_workspace_group() {
    let mut records = seeded();
    records.push(record(
        "cli:wt",
        "local worktree work",
        Some(6),
        git("/work/wt", "quecto"),
    ));
    let rig = rig_at(HERE, Metadata::of(records.clone()));
    assert_eq!(
        keys(&rig, "work", SessionListScope::Local).await,
        ["cli:wt", "cli:mine"]
    );
    let rig = rig_at(HERE, Metadata::of(records.clone()));
    assert_eq!(
        keys(&rig, "zebra", SessionListScope::Local).await,
        Vec::<String>::new()
    );
    // The same query, globally, also reaches paths under /work elsewhere.
    let rig = rig_at(HERE, Metadata::of(records));
    assert_eq!(keys(&rig, "work", SessionListScope::Global).await.len(), 5);
}

#[tokio::test]
async fn without_workspace_facts_local_never_broadens_and_global_still_answers() {
    let rig = rig_at("/nowhere", Metadata::of(seeded()));
    let request = SearchSessionMetadataRequest {
        query: "work".into(),
        ..Default::default()
    };
    let result = rig.search.search(&request).await.unwrap();
    assert_eq!(request.scope, SessionListScope::Local);
    assert!(result.rows.is_empty() && result.searched == 0);
    assert!(
        result
            .freshness
            .diagnostics
            .iter()
            .any(|d| d.contains("not discoverable"))
    );
    let rig = rig_at("/nowhere", Metadata::of(seeded()));
    let rows = keyed(&rig, "work", SessionListScope::Global).await.rows;
    assert!(!rows.is_empty() && rows.iter().all(|row| !row.session.resume_eligible));
}

#[tokio::test]
async fn order_is_rank_then_newest_then_key_and_the_limit_reports_what_it_left_out() {
    let records = vec![
        record("k-path-new", "x", Some(99), git("/work/otter/p", "r1")),
        record("k-repo", "x", Some(1), git("/w/p", "otter")),
        record(
            "k-title-old",
            "otter a",
            Some(1),
            SessionHomeScope::LegacyUnscoped,
        ),
        record(
            "k-title-b",
            "otter b",
            Some(9),
            SessionHomeScope::LegacyUnscoped,
        ),
        record(
            "k-title-a",
            "otter c",
            Some(9),
            SessionHomeScope::LegacyUnscoped,
        ),
        record(
            "k-title-undated",
            "otter d",
            None,
            SessionHomeScope::LegacyUnscoped,
        ),
        record(
            "otter",
            "the key itself",
            Some(0),
            SessionHomeScope::LegacyUnscoped,
        ),
    ];
    let expected = [
        "otter",
        "k-title-a",
        "k-title-b",
        "k-title-old",
        "k-title-undated",
        "k-repo",
        "k-path-new",
    ];
    let mut shuffled = records.clone();
    shuffled.reverse();
    for input in [records, shuffled] {
        let rig = rig_at(HERE, Metadata::of(input.clone()));
        assert_eq!(
            keys(&rig, "otter", SessionListScope::Global).await,
            expected
        );
        let rig = rig_at(HERE, Metadata::of(input));
        let request = SearchSessionMetadataRequest {
            query: "otter".into(),
            scope: SessionListScope::Global,
            limit: SearchLimit::clamped(Some(3)),
            ..Default::default()
        };
        let result = rig.search.search(&request).await.unwrap();
        let shown: Vec<_> = result
            .rows
            .iter()
            .map(|r| r.session.summary.key.as_str())
            .collect();
        assert_eq!(shown, expected[..3]);
        assert_eq!((result.total_matches, result.truncated()), (7, true));
    }
}

#[tokio::test]
async fn a_query_with_nothing_visible_lists_the_scope_newest_first() {
    let rig = rig_at(HERE, Metadata::of(seeded()));
    let all = keys(&rig, " \u{200b} ", SessionListScope::Global).await;
    assert_eq!(
        all,
        [
            "cli:legacy",
            "chat-1-title",
            "chat-2-key",
            "chat-3-repo",
            "chat-4-path",
            "cli:mine"
        ]
    );
}

#[tokio::test]
async fn a_refused_query_is_answered_without_asking_the_catalogue_or_git() {
    let rig = rig_at(HERE, Metadata::of(seeded()));
    let request = SearchSessionMetadataRequest {
        query: "z".repeat(257),
        scope: SessionListScope::Global,
        generation: QueryGeneration(11),
        ..Default::default()
    };
    let result = rig.search.search(&request).await.unwrap();
    assert_eq!(result.refused, Some(QueryRefusal::TooLong { chars: 257 }));
    assert!(result.rows.is_empty() && result.total_matches == 0 && result.searched == 0);
    assert_eq!(result.generation, QueryGeneration(11));
    assert_eq!(*rig.catalogue.queries.lock().unwrap(), 0);
    assert!(rig.discovery.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn eligibility_is_the_domain_rule_over_one_discovery_of_this_directory() {
    let mut records = seeded();
    records.push(record(
        "cli:wt",
        "work in a worktree",
        Some(6),
        git("/work/wt", "quecto"),
    ));
    let changed = SessionHomeScope::Scoped(SessionHome {
        execution_dir: HERE.into(),
        group: WorkspaceGroup::Folder {
            directory: HERE.into(),
        },
        provenance: AssociationProvenance::SavedHere,
    });
    records.push(record(
        "cli:was-a-folder",
        "work before git init",
        Some(4),
        changed,
    ));
    let rig = rig_at(HERE, Metadata::of(records));
    let eligible: Vec<_> = keyed(&rig, "work", SessionListScope::Global)
        .await
        .rows
        .into_iter()
        .filter(|row| row.session.resume_eligible)
        .map(|row| row.session.summary.key)
        .collect();
    assert_eq!(eligible, ["cli:mine"], "same directory AND same group only");
    assert_eq!(
        *rig.discovery.0.lock().unwrap(),
        [PathBuf::from(HERE)],
        "no discovery per row"
    );
}

#[tokio::test]
async fn freshness_passes_through_and_a_catalogue_failure_is_the_answer() {
    let rig = rig_at(HERE, Metadata::of(seeded()));
    let result = rig
        .search
        .search(&SearchSessionMetadataRequest::default())
        .await
        .unwrap();
    assert!(result.freshness.rebuilt);
    assert_eq!(
        result.freshness.diagnostics,
        ["cli_bad.json: session record unavailable"]
    );
    let failing = Metadata::answering(Err(DomainError::Session("sessions dir unreadable".into())));
    let rig = rig_at(HERE, failing);
    let error = rig
        .search
        .search(&SearchSessionMetadataRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "session error: sessions dir unreadable");
}

#[tokio::test]
async fn a_row_names_its_repository_and_debug_prints_no_context() {
    let rig = rig_at(HERE, Metadata::of(seeded()));
    let result = keyed(&rig, "walrus", SessionListScope::Global).await;
    assert_eq!(result.rows[0].repository_label.as_deref(), Some("walrus"));
    assert_eq!(format!("{:?}", rig.search), "SearchSessionMetadata { .. }");
}
