use super::*;
use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, RepositoryLabel, RepositoryWorktreeGrouping, SessionHomeScope,
};
use crate::domain::session_identity::SessionIdentity;

fn row(
    key: &str,
    title: &str,
    scope: SessionHomeScope,
    legacy: bool,
) -> ScopedSessionRow {
    let (repo, path) = match &scope {
        SessionHomeScope::Scoped {
            execution_location,
            repository_grouping,
        } => (
            repository_grouping
                .as_ref()
                .map(|g| g.repository_label().clone()),
            Some(execution_location.clone()),
        ),
        SessionHomeScope::LegacyUnscoped => (None, None),
    };
    ScopedSessionRow {
        identity: SessionIdentity::from_persisted_key(key),
        title: title.into(),
        message_count: 1,
        updated_unix_secs: Some(1),
        home_scope: scope,
        repository_label: repo,
        execution_path: path,
        is_legacy_unscoped: legacy,
    }
}

#[test]
fn global_query_matches_title_key_repo_path_not_requiring_body() {
    let loc = CanonicalExecutionLocation::from_canonical_path("/work/quecto");
    let scope = SessionHomeScope::scoped(
        loc.clone(),
        Some(RepositoryWorktreeGrouping::related_worktrees(
            RepositoryLabel::new("quecto").unwrap(),
            loc,
            vec![CanonicalExecutionLocation::from_canonical_path("/work/quecto")],
        )),
    );
    let r = row("chat-1-abc", "fix the picker", scope, false);
    assert!(row_matches_metadata_query(&r, "picker"));
    assert!(row_matches_metadata_query(&r, "chat-1"));
    assert!(row_matches_metadata_query(&r, "quecto"));
    assert!(row_matches_metadata_query(&r, "/work/"));
    assert!(!row_matches_metadata_query(&r, "missing-token"));
    assert!(row_matches_metadata_query(&r, ""));
}

#[test]
fn local_scope_excludes_legacy_unscoped() {
    let current = SessionHomeScope::scoped(
        CanonicalExecutionLocation::from_canonical_path("/a"),
        None,
    );
    let legacy = row("cli:old", "legacy", SessionHomeScope::LegacyUnscoped, true);
    assert!(!row_matches_local_scope(&legacy, &current));
}

#[test]
fn local_scope_includes_same_execution_and_related_worktree() {
    let main = CanonicalExecutionLocation::from_canonical_path("/repo/main");
    let wt = CanonicalExecutionLocation::from_canonical_path("/repo/wt");
    let grouping = RepositoryWorktreeGrouping::related_worktrees(
        RepositoryLabel::new("repo").unwrap(),
        main.clone(),
        vec![main.clone(), wt.clone()],
    );
    let current = SessionHomeScope::scoped(main.clone(), Some(grouping.clone()));
    let same = row(
        "chat-same",
        "t",
        SessionHomeScope::scoped(main, Some(grouping.clone())),
        false,
    );
    let related = row(
        "chat-wt",
        "t",
        SessionHomeScope::scoped(wt, Some(grouping)),
        false,
    );
    let other = row(
        "chat-other",
        "t",
        SessionHomeScope::scoped(CanonicalExecutionLocation::from_canonical_path("/other"), None),
        false,
    );
    assert!(row_matches_local_scope(&same, &current));
    assert!(row_matches_local_scope(&related, &current));
    assert!(!row_matches_local_scope(&other, &current));
}

#[test]
fn scope_list_query_constructors() {
    let q = ScopeListQuery::global("x");
    assert!(q.is_global());
    assert!(!q.is_local());
    let local = ScopeListQuery::local(SessionHomeScope::LegacyUnscoped);
    assert!(local.is_local());
}
