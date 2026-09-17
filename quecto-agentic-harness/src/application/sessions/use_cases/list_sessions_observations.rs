//! Query-local Git discovery cache for listing: eligibility by the one
//! domain rule over observations that live for one query only. Resume
//! still admits under its claim; nothing here is authority.
use std::collections::HashMap;
use std::path::PathBuf;

use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::session_home::{HomeAdmission, SessionHome, SessionHomeScope};

pub(super) fn seed_observations(
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
pub(super) async fn resume_eligible(
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
