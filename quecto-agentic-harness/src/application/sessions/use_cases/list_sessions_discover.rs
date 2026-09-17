//! Query-local catalogue projection and Git discovery cache for listing.
//! Resume still admits under its claim; observations here are advisory
//! and live for one query only.
use std::collections::HashMap;
use std::path::PathBuf;

use crate::application::sessions::dto::{ListSessionsResult, ListedSession, SessionListScope};
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::{SessionHome, SessionHomeScope};
use crate::domain::session_identity::SessionIdentity;

pub(super) async fn discover(
    home: Option<&SessionHomeContext>,
    summaries: Vec<SessionSummary>,
    scope: SessionListScope,
) -> ListSessionsResult {
    let mut result = ListSessionsResult {
        sessions: Vec::new(),
        diagnostics: Vec::new(),
        rebuilt: false,
    };
    let current = current_home(home, &mut result.diagnostics);
    let (catalogue_available, entries) = catalogue_entries(home, &mut result).await;
    // Query-local observations only: resume still performs fresh admission under
    // its claim. Cache failures too, and reuse the current canonical observation.
    let mut observations = seed_observations(home, current.as_ref());
    for summary in summaries {
        let Some(listed) = project_row(
            summary,
            &entries,
            catalogue_available,
            current.as_ref(),
            home,
            scope,
            &mut observations,
        ) else {
            continue;
        };
        result.sessions.push(listed);
    }
    result
}

fn current_home(
    home: Option<&SessionHomeContext>,
    diagnostics: &mut Vec<String>,
) -> Option<SessionHome> {
    match home {
        Some(context) => match context.current() {
            Ok(home) => Some(home),
            Err(error) => {
                diagnostics.push(error.to_string());
                None
            }
        },
        None => {
            diagnostics.push("workspace discovery unavailable".into());
            None
        }
    }
}

async fn catalogue_entries(
    home: Option<&SessionHomeContext>,
    result: &mut ListSessionsResult,
) -> (bool, Vec<(SessionIdentity, SessionHomeScope)>) {
    match home {
        Some(context) => match context.catalogue.list_async().await {
            Ok(snapshot) => {
                result.diagnostics.extend(snapshot.diagnostics);
                result.rebuilt = snapshot.rebuilt;
                (true, snapshot.entries)
            }
            Err(error) => {
                result.diagnostics.push(error.to_string());
                (false, Vec::new())
            }
        },
        None => (false, Vec::new()),
    }
}

fn seed_observations(
    context: Option<&SessionHomeContext>,
    current: Option<&SessionHome>,
) -> HashMap<PathBuf, Option<SessionHome>> {
    let mut observations = HashMap::new();
    if let (Some(context), Some(current)) = (context, current) {
        observations.insert(context.execution_dir.clone(), Some(current.clone()));
        observations.insert(current.execution_dir.clone(), Some(current.clone()));
    }
    observations
}

fn project_row(
    summary: SessionSummary,
    entries: &[(SessionIdentity, SessionHomeScope)],
    catalogue_available: bool,
    current: Option<&SessionHome>,
    context: Option<&SessionHomeContext>,
    scope: SessionListScope,
    observations: &mut HashMap<PathBuf, Option<SessionHome>>,
) -> Option<ListedSession> {
    let home = match entries
        .iter()
        .find(|(identity, _)| identity.runtime_key() == summary.key)
    {
        Some((_, home)) => home.clone(),
        None if catalogue_available => return None,
        None => SessionHomeScope::Unavailable("authoritative home unavailable".into()),
    };
    let local = matches!(
        (&home, current),
        (SessionHomeScope::Scoped(home), Some(current)) if home.group == current.group
    );
    if scope == SessionListScope::Global || local {
        Some(ListedSession {
            summary,
            home: home.clone(),
            resume_eligible: resume_eligible(context, current, &home, observations),
        })
    } else {
        None
    }
}

fn resume_eligible(
    context: Option<&SessionHomeContext>,
    current: Option<&SessionHome>,
    home: &SessionHomeScope,
    observations: &mut HashMap<PathBuf, Option<SessionHome>>,
) -> bool {
    match (context, current, home) {
        (Some(context), Some(current), SessionHomeScope::Scoped(saved)) => observations
            .entry(saved.execution_dir.clone())
            .or_insert_with(|| context.discovery.discover(&saved.execution_dir).ok())
            .as_ref()
            .is_some_and(|observed| observed == saved && observed.same_execution(current)),
        _ => false,
    }
}
