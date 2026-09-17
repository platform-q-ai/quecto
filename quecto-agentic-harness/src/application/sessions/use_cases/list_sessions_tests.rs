use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::sessions::dto::{ListSessionsRequest, SessionListQuery, SessionListScope};
use crate::domain::session::Session;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::SessionHomeScope;
use crate::domain::session_identity::{SessionIdentity, SessionKeyPrefix};

/// A store fake that records the queries it receives and answers with a
/// scripted outcome; it never loads a session (list is summary-only).
struct ScriptedStore {
    queries: Mutex<Vec<SessionListQuery>>,
    outcome: Mutex<Option<Result<Vec<SessionSummary>, DomainError>>>,
    loads: Mutex<usize>,
}

impl ScriptedStore {
    fn answering(outcome: Result<Vec<SessionSummary>, DomainError>) -> Arc<Self> {
        Arc::new(Self {
            queries: Mutex::new(Vec::new()),
            outcome: Mutex::new(Some(outcome)),
            loads: Mutex::new(0),
        })
    }
}

impl SessionStore for ScriptedStore {
    fn load(
        &self,
        _identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
        *self.loads.lock().unwrap() += 1;
        Box::pin(async { Ok(None) })
    }

    fn save(
        &self,
        _session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn exists(
        &self,
        _identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }

    fn list(
        &self,
        query: &SessionListQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        self.queries.lock().unwrap().push(query.clone());
        let outcome = self.outcome.lock().unwrap().take();
        Box::pin(async move { outcome.expect("one list per scripted store") })
    }
}

fn summary(key: &str, updated: Option<u64>) -> SessionSummary {
    SessionSummary {
        key: key.to_string(),
        title: format!("title of {key}"),
        message_count: 2,
        updated_unix_secs: updated,
    }
}

fn global(query: SessionListQuery) -> ListSessionsRequest {
    ListSessionsRequest {
        query,
        scope: SessionListScope::Global,
    }
}

#[tokio::test]
async fn all_query_returns_the_store_summaries_in_store_order() {
    // The adapter promises newest-first; the query passes that order through
    // untouched (no re-sort, no filtering) and performs no full loads.
    let newest_first = vec![
        summary("chat-3", Some(30)),
        summary("cli:two", Some(20)),
        summary("chat-1", Some(10)),
        summary("cli:undated", None),
    ];
    let store = ScriptedStore::answering(Ok(newest_first.clone()));
    let query = ListSessions::new(store.clone());

    let listed = query
        .discover(&global(SessionListQuery::All))
        .await
        .unwrap();

    let summaries: Vec<_> = listed.sessions.into_iter().map(|row| row.summary).collect();
    assert_eq!(summaries, newest_first);
    assert_eq!(*store.queries.lock().unwrap(), vec![SessionListQuery::All]);
    assert_eq!(*store.loads.lock().unwrap(), 0, "list is summary-only");
}

#[tokio::test]
async fn prefix_query_is_handed_to_the_store_as_the_identity_prefix() {
    let store = ScriptedStore::answering(Ok(vec![summary("chat-9", Some(9))]));
    let query = ListSessions::new(store.clone());
    let prefix = SessionKeyPrefix::new("chat-").unwrap();

    let listed = query
        .discover(&global(SessionListQuery::ExistingKeyPrefix(prefix.clone())))
        .await
        .unwrap();

    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(
        *store.queries.lock().unwrap(),
        vec![SessionListQuery::ExistingKeyPrefix(prefix)]
    );
}

#[tokio::test]
async fn empty_store_lists_nothing() {
    let store = ScriptedStore::answering(Ok(Vec::new()));
    let listed = ListSessions::new(store)
        .discover(&global(SessionListQuery::All))
        .await
        .unwrap();
    assert!(listed.sessions.is_empty());
}

#[tokio::test]
async fn store_error_surfaces_unchanged() {
    let store = ScriptedStore::answering(Err(DomainError::Session(
        "failed to read sessions dir: boom".to_string(),
    )));
    let err = ListSessions::new(store)
        .discover(&global(SessionListQuery::All))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "session error: failed to read sessions dir: boom"
    );
}

#[test]
fn debug_does_not_print_the_store() {
    let store = ScriptedStore::answering(Ok(Vec::new()));
    assert_eq!(
        format!("{:?}", ListSessions::new(store)),
        "ListSessions { .. }"
    );
}

#[tokio::test]
async fn local_discovery_without_workspace_facts_never_broadens_to_global() {
    let store = ScriptedStore::answering(Ok(vec![summary("chat-foreign", Some(1))]));
    let listed = ListSessions::new(store)
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Local,
        })
        .await
        .unwrap();
    assert!(listed.sessions.is_empty());
    assert!(!listed.diagnostics.is_empty());
}

use crate::application::sessions::ports::session_home::{
    HomeCatalogueSnapshot, SessionHomeCatalogue, WorkspaceDiscovery,
};
use crate::domain::session_home::{AssociationProvenance, SessionHome, WorkspaceGroup};
use std::path::{Path, PathBuf};

struct CountingDiscovery(Mutex<Vec<PathBuf>>);
impl WorkspaceDiscovery for CountingDiscovery {
    fn discover(&self, path: &Path) -> Result<SessionHome, DomainError> {
        self.0.lock().unwrap().push(path.into());
        match path.to_str() {
            Some("/missing") => Err(DomainError::Session("missing directory".into())),
            _ => Ok(observed_home(path)),
        }
    }
}

fn observed_home(path: &Path) -> SessionHome {
    SessionHome {
        execution_dir: path.into(),
        group: WorkspaceGroup::Git {
            common_dir: "/repo/.git".into(),
        },
        provenance: AssociationProvenance::SavedHere,
    }
}

struct FixedCatalogue(Vec<(SessionIdentity, SessionHomeScope)>);
impl SessionHomeCatalogue for FixedCatalogue {
    fn read(&self, _: &SessionIdentity) -> Result<SessionHomeScope, DomainError> {
        panic!("listing must use summary catalogue")
    }
    fn record_new(&self, _: &SessionIdentity, _: &SessionHome) -> Result<(), DomainError> {
        panic!("listing must not mutate authority")
    }
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError> {
        Ok(HomeCatalogueSnapshot {
            entries: self.0.clone(),
            diagnostics: vec![],
            rebuilt: false,
        })
    }
}

#[tokio::test]
async fn discovery_observations_scale_with_directories_not_rows() {
    let scopes = [
        SessionHomeScope::Scoped(observed_home(Path::new("/here"))),
        SessionHomeScope::Scoped(observed_home(Path::new("/elsewhere"))),
        SessionHomeScope::Scoped(observed_home(Path::new("/missing"))),
        SessionHomeScope::LegacyUnscoped,
        SessionHomeScope::Unavailable("broken authority".into()),
    ];
    let entries: Vec<_> = (0..100)
        .map(|i| {
            (
                SessionIdentity::user_chat(&format!("chat-{i}")).unwrap(),
                scopes[i % scopes.len()].clone(),
            )
        })
        .collect();
    let summaries = entries
        .iter()
        .map(|(id, _)| summary(id.runtime_key(), None))
        .collect();
    let discovery = Arc::new(CountingDiscovery(Mutex::new(vec![])));
    let listed = ListSessions::new(ScriptedStore::answering(Ok(summaries)))
        .with_home(SessionHomeContext {
            catalogue: Arc::new(FixedCatalogue(entries)),
            discovery: discovery.clone(),
            execution_dir: Ok("/here".into()),
        })
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Global,
        })
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 100);
    for (i, row) in listed.sessions.iter().enumerate() {
        assert_eq!(row.resume_eligible, i % scopes.len() == 0);
    }
    assert_eq!(
        *discovery.0.lock().unwrap(),
        vec![
            PathBuf::from("/here"),
            PathBuf::from("/elsewhere"),
            PathBuf::from("/missing")
        ]
    );
}

#[tokio::test]
async fn one_hundred_sessions_in_one_directory_discover_once() {
    let saved = observed_home(Path::new("/here"));
    let entries: Vec<_> = (0..100)
        .map(|i| {
            (
                SessionIdentity::user_chat(&format!("chat-same-{i}")).unwrap(),
                SessionHomeScope::Scoped(saved.clone()),
            )
        })
        .collect();
    let summaries = entries
        .iter()
        .map(|(id, _)| summary(id.runtime_key(), None))
        .collect();
    let discovery = Arc::new(CountingDiscovery(Mutex::new(vec![])));
    let listed = ListSessions::new(ScriptedStore::answering(Ok(summaries)))
        .with_home(SessionHomeContext {
            catalogue: Arc::new(FixedCatalogue(entries)),
            discovery: discovery.clone(),
            execution_dir: Ok("/here".into()),
        })
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Global,
        })
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 100);
    assert!(listed.sessions.iter().all(|row| row.resume_eligible));
    assert_eq!(
        *discovery.0.lock().unwrap(),
        vec![PathBuf::from("/here")],
        "shared Git directory must not rediscover per row"
    );
}

#[tokio::test]
async fn current_canonical_observation_is_reused_for_saved_directory() {
    struct CanonicalDiscovery(Mutex<Vec<PathBuf>>);
    impl WorkspaceDiscovery for CanonicalDiscovery {
        fn discover(&self, path: &Path) -> Result<SessionHome, DomainError> {
            self.0.lock().unwrap().push(path.into());
            let observed = match path.to_str() {
                Some("/cwd") => Path::new("/canonical"),
                _ => path,
            };
            Ok(observed_home(observed))
        }
    }
    let saved = observed_home(Path::new("/canonical"));
    let entries: Vec<_> = (0..100)
        .map(|i| {
            (
                SessionIdentity::user_chat(&format!("chat-canon-{i}")).unwrap(),
                SessionHomeScope::Scoped(saved.clone()),
            )
        })
        .collect();
    let summaries = entries
        .iter()
        .map(|(id, _)| summary(id.runtime_key(), None))
        .collect();
    let discovery = Arc::new(CanonicalDiscovery(Mutex::new(vec![])));
    let listed = ListSessions::new(ScriptedStore::answering(Ok(summaries)))
        .with_home(SessionHomeContext {
            catalogue: Arc::new(FixedCatalogue(entries)),
            discovery: discovery.clone(),
            execution_dir: Ok("/cwd".into()),
        })
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Global,
        })
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 100);
    assert!(listed.sessions.iter().all(|row| row.resume_eligible));
    assert_eq!(
        *discovery.0.lock().unwrap(),
        vec![PathBuf::from("/cwd")],
        "canonical current facts must cover saved execution directories"
    );
}

#[tokio::test]
async fn local_scope_skips_discovery_for_foreign_and_non_scoped_homes() {
    fn folder(path: &str) -> SessionHome {
        SessionHome {
            execution_dir: path.into(),
            group: WorkspaceGroup::Folder {
                directory: path.into(),
            },
            provenance: AssociationProvenance::SavedHere,
        }
    }
    let entries = vec![
        (
            SessionIdentity::user_chat("chat-here").unwrap(),
            SessionHomeScope::Scoped(observed_home(Path::new("/here"))),
        ),
        (
            SessionIdentity::user_chat("chat-foreign").unwrap(),
            SessionHomeScope::Scoped(folder("/foreign")),
        ),
        (
            SessionIdentity::user_chat("chat-legacy").unwrap(),
            SessionHomeScope::LegacyUnscoped,
        ),
        (
            SessionIdentity::user_chat("chat-broken").unwrap(),
            SessionHomeScope::Unavailable("broken".into()),
        ),
    ];
    let summaries = entries
        .iter()
        .map(|(id, _)| summary(id.runtime_key(), None))
        .collect();
    let discovery = Arc::new(CountingDiscovery(Mutex::new(vec![])));
    let listed = ListSessions::new(ScriptedStore::answering(Ok(summaries)))
        .with_home(SessionHomeContext {
            catalogue: Arc::new(FixedCatalogue(entries)),
            discovery: discovery.clone(),
            execution_dir: Ok("/here".into()),
        })
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Local,
        })
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].summary.key, "chat-here");
    assert!(listed.sessions[0].resume_eligible);
    assert_eq!(
        *discovery.0.lock().unwrap(),
        vec![PathBuf::from("/here")],
        "local listing must not discover foreign, legacy, or unavailable homes"
    );
}

#[tokio::test]
async fn discovery_observations_are_fresh_each_query_and_failure_never_admits() {
    let identity = SessionIdentity::user_chat("chat-fresh").unwrap();
    let saved = observed_home(Path::new("/here"));
    let store = ScriptedStore::answering(Ok(vec![summary(identity.runtime_key(), None)]));
    let discovery = Arc::new(CountingDiscovery(Mutex::new(vec![])));
    let context = SessionHomeContext {
        catalogue: Arc::new(FixedCatalogue(vec![(
            identity.clone(),
            SessionHomeScope::Scoped(saved.clone()),
        )])),
        discovery: discovery.clone(),
        execution_dir: Ok("/here".into()),
    };
    let query = ListSessions::new(store.clone()).with_home(context.clone());
    let request = ListSessionsRequest {
        query: SessionListQuery::All,
        scope: SessionListScope::Global,
    };
    for _ in 0..2 {
        *store.outcome.lock().unwrap() = Some(Ok(vec![summary(identity.runtime_key(), None)]));
        assert!(query.discover(&request).await.unwrap().sessions[0].resume_eligible);
    }
    assert_eq!(discovery.0.lock().unwrap().len(), 2);
    // Advisory query caching does not change the authoritative context's checks.
    assert!(context.eligible(&SessionHomeScope::Scoped(saved)).await);
    assert_eq!(discovery.0.lock().unwrap().len(), 4);
    *store.outcome.lock().unwrap() = Some(Ok(vec![summary(identity.runtime_key(), None)]));
    let unavailable = ListSessions::new(store)
        .with_home(SessionHomeContext {
            execution_dir: Ok("/missing".into()),
            ..context
        })
        .discover(&request)
        .await
        .unwrap();
    assert!(!unavailable.sessions[0].resume_eligible);
    assert_eq!(unavailable.diagnostics.len(), 1);
    assert_eq!(
        discovery.0.lock().unwrap().len(),
        5,
        "failed current discovery skips row observations"
    );
}
