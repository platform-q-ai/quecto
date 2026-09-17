//! Query-local catalogue projection and Git discovery cache for listing.
//! Resume still admits under its claim; observations here are advisory
//! and live for one query only.
use std::collections::HashMap;
use std::path::PathBuf;

use crate::application::sessions::dto::{ListSessionsResult, ListedSession, SessionListScope};
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::{HomeAdmission, SessionHome, SessionHomeScope};

/// Catalogue rows keyed by runtime key; `None` when authority is unavailable.
type CatalogueRows = Option<HashMap<String, SessionHomeScope>>;

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
    let current = current_home(home, &mut result.diagnostics).await;
    let rows = catalogue_rows(home, &mut result).await;
    // Query-local observations only: resume still performs fresh admission under
    // its claim. Cache failures too, and reuse the current canonical observation.
    let mut observations = seed_observations(home, current.as_ref());
    for summary in summaries {
        let home_scope = match &rows {
            // A store-listed record the strict catalogue rejected (a crash-
            // truncated transcript) stays visible globally and never eligible.
            Some(rows) => rows
                .get(&summary.key)
                .cloned()
                .unwrap_or_else(|| SessionHomeScope::Unavailable("record not in catalogue".into())),
            None => SessionHomeScope::Unavailable("authoritative home unavailable".into()),
        };
        let local = matches!(
            (&home_scope, &current),
            (SessionHomeScope::Scoped(saved), Some(current)) if saved.group == current.group
        );
        if scope == SessionListScope::Global || local {
            let resume_eligible =
                resume_eligible(home, current.as_ref(), &home_scope, &mut observations).await;
            result.sessions.push(ListedSession {
                summary,
                home: home_scope,
                resume_eligible,
            });
        }
    }
    result
}

async fn current_home(
    home: Option<&SessionHomeContext>,
    diagnostics: &mut Vec<String>,
) -> Option<SessionHome> {
    match home {
        Some(context) => match context.current().await {
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

async fn catalogue_rows(
    home: Option<&SessionHomeContext>,
    result: &mut ListSessionsResult,
) -> CatalogueRows {
    match home {
        Some(context) => match context.catalogue.list_async().await {
            Ok(snapshot) => {
                result.diagnostics.extend(snapshot.diagnostics);
                result.rebuilt = snapshot.rebuilt;
                Some(
                    snapshot
                        .entries
                        .into_iter()
                        .map(|(identity, home)| (identity.runtime_key().to_string(), home))
                        .collect(),
                )
            }
            Err(error) => {
                result.diagnostics.push(error.to_string());
                None
            }
        },
        None => None,
    }
}

fn seed_observations(
    context: Option<&SessionHomeContext>,
    current: Option<&SessionHome>,
) -> HashMap<PathBuf, Option<SessionHome>> {
    let mut observations = HashMap::new();
    if let (Some(context), Some(current)) = (context, current) {
        if let Ok(execution_dir) = &context.execution_dir {
            observations.insert(execution_dir.clone(), Some(current.clone()));
        }
        observations.insert(current.execution_dir.clone(), Some(current.clone()));
    }
    observations
}

/// Advisory eligibility by the one domain rule, over cached observations.
async fn resume_eligible(
    context: Option<&SessionHomeContext>,
    current: Option<&SessionHome>,
    home: &SessionHomeScope,
    observations: &mut HashMap<PathBuf, Option<SessionHome>>,
) -> bool {
    let (Some(context), Some(current), SessionHomeScope::Scoped(saved)) = (context, current, home)
    else {
        return false;
    };
    if !observations.contains_key(&saved.execution_dir) {
        let observed = context
            .discovery
            .discover_async(&saved.execution_dir)
            .await
            .ok();
        observations.insert(saved.execution_dir.clone(), observed);
    }
    observations
        .get(&saved.execution_dir)
        .and_then(Option::as_ref)
        .is_some_and(|observed| {
            SessionHome::admission(saved, observed, current) == HomeAdmission::Eligible
        })
}
