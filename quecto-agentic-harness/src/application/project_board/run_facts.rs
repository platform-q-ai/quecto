//! What a client saw of a run, sent with its end to a board server (epic
//! #2705 B8), and the ports an end reads on the server, stated: gathered
//! before the write, so the server's transaction asks no front end, git
//! or host anything. A client's facts are trusted cooperatively, as the
//! local board trusts its own front end ([`super::ports::ServedRun`]); the
//! remote's revisions are read by the server itself, before its lock.
use std::collections::BTreeSet;
use std::sync::Arc;

use super::dto::SwarmRunView;
use super::fold_back::evidence_revisions;
use super::ports::{BoardStoreError, RemoteRevisions, RunLiveness, SwarmRunReports};
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::task_parts::{MAX_ITEMS, MAX_PER_ITEM};
use crate::domain::project_board::value_objects::commit_token::checks_commit;
use crate::domain::project_board::value_objects::slug::Slug;

/// Whether a client's front end looked for a run's coordinator.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SeenLiveness {
    /// It sees no swarm (the CLI), so it looked for none.
    Unseen,
    /// It looked: the coordinator still running there, if one is.
    Seen(Option<String>),
}

/// A run's swarm boards as a client kept them: as its swarm ended, and as
/// its owner left it running (#2674).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeptViews {
    pub ended: Option<SwarmRunView>,
    pub left: Option<SwarmRunView>,
}

/// What a client saw of the run its end names, before it sent the end; on
/// the wire as it is (an end without facts saw and kept nothing).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFacts {
    pub liveness: SeenLiveness,
    /// `None`: it keeps no swarm boards (the CLI).
    pub reports: Option<KeptViews>,
}

impl RunFacts {
    /// What `liveness` and `reports` (`None`: this front end has none) say
    /// of run `run` of `task` now.
    pub fn gathered(
        (task, run): (&Slug, u32),
        liveness: Option<&dyn RunLiveness>,
        reports: Option<&dyn SwarmRunReports>,
    ) -> Self {
        let seen = |liveness: &dyn RunLiveness| liveness.running_coordinator(task, run);
        let kept = |reports: &dyn SwarmRunReports| KeptViews {
            ended: reports.ended_run(task, run),
            left: reports.left_run(task, run),
        };
        Self {
            liveness: liveness.map_or(SeenLiveness::Unseen, |l| SeenLiveness::Seen(seen(l))),
            reports: reports.map(kept),
        }
    }

    /// The facts of a client that sent none: it saw and kept nothing.
    pub fn unseen() -> Self {
        Self {
            liveness: SeenLiveness::Unseen,
            reports: None,
        }
    }

    /// The liveness an end of run `run` of `task` reads on the server;
    /// `None` as the client saw none.
    pub fn liveness(&self, (task, run): (&Slug, u32)) -> Option<Arc<dyn RunLiveness>> {
        match &self.liveness {
            SeenLiveness::Unseen => None,
            SeenLiveness::Seen(agent) => Some(Arc::new(StatedRunLiveness {
                run: (task.clone(), run),
                agent: agent.clone(),
            })),
        }
    }

    /// The swarm boards an end of run `run` of `task` reads on the
    /// server; `None` as the client kept none.
    pub fn reports(&self, (task, run): (&Slug, u32)) -> Option<Arc<dyn SwarmRunReports>> {
        let views = self.reports.clone()?;
        let run = (task.clone(), run);
        Some(Arc::new(StatedSwarmRunReports { run, views }))
    }

    /// The commits the evidence of `plan`'s items names on the board its
    /// swarm ended on (as a success records them), once each and at most
    /// `MAX_ITEMS × MAX_PER_ITEM`: what the server reads the remote for
    /// before its lock. A revision past them is unread, and refuses.
    pub fn named_commits(&self, plan: &Plan) -> Vec<String> {
        let ended = self.reports.as_ref().and_then(|views| views.ended.as_ref());
        let named = ended.map(|view| evidence_revisions(plan, view).ids);
        let mut named = named.unwrap_or_default();
        named.truncate(MAX_ITEMS * MAX_PER_ITEM * 2);
        named
    }

    /// These facts within `bytes` on the wire: as they are; else, for an
    /// end that is no success, its boards without their evidence (as a
    /// run past the board's bytes is recorded); else without its boards. A
    /// success keeps its evidence, which the push rule checks, or no board.
    pub fn fitting(mut self, succeeds: bool, bytes: usize) -> Self {
        let size = |facts: &Self| serde_json::to_vec(facts).map_or(usize::MAX, |wire| wire.len());
        match (size(&self) <= bytes || bytes > 0, succeeds) {
            (true, _) => return self,
            (false, false) => {
                let views = self.reports.iter_mut();
                let views = views.flat_map(|views| [&mut views.ended, &mut views.left]);
                let items = views.flatten().flat_map(|view| view.items.iter_mut());
                items.for_each(|item| item.evidence.clear());
            }
            (false, true) => {}
        }
        match size(&self) <= bytes {
            true => self,
            false => Self {
                reports: None,
                ..self
            },
        }
    }
}

/// Whether a run's coordinator still ran where its client looked.
pub struct StatedRunLiveness {
    run: (Slug, u32),
    agent: Option<String>,
}

impl RunLiveness for StatedRunLiveness {
    fn running_coordinator(&self, task: &Slug, run: u32) -> Option<String> {
        match (task, run) == (&self.run.0, self.run.1) {
            true => self.agent.clone(),
            false => None,
        }
    }
}

/// A run's swarm boards as its client kept them; recording them is the
/// client's, once the server answers its end.
pub struct StatedSwarmRunReports {
    run: (Slug, u32),
    views: KeptViews,
}

impl StatedSwarmRunReports {
    fn of(&self, task: &Slug, run: u32, view: &Option<SwarmRunView>) -> Option<SwarmRunView> {
        let this = (task, run) == (&self.run.0, self.run.1);
        view.clone().filter(|_| this)
    }
}

impl SwarmRunReports for StatedSwarmRunReports {
    fn ended_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView> {
        self.of(task, run, &self.views.ended)
    }

    fn left_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView> {
        self.of(task, run, &self.views.left)
    }

    fn recorded(&self, _: &Slug, _: u32) {}
}

/// Which revisions the board's remote held as a write was prepared: its
/// commit ids' length, the revisions asked about, and those it did not
/// hold; or why it was not read. Any other revision is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatedRemoteRevisions(Result<Judged, BoardStoreError>);

/// A remote's commit ids' length, the revisions asked about, and those of
/// them it did not hold.
type Judged = (usize, BTreeSet<String>, BTreeSet<String>);

impl StatedRemoteRevisions {
    /// `named` as `remote` judges them now: those that name a commit its
    /// ids can name ([`checks_commit`]) asked about once.
    pub fn read(remote: &dyn RemoteRevisions, named: &[String]) -> Self {
        let read = || -> Result<Judged, BoardStoreError> {
            let id_len = remote.commit_id_hex_len()?;
            let checked = |id: &&String| checks_commit(id, id_len);
            let asked: Vec<String> = named.iter().filter(checked).cloned().collect();
            let missing = match asked.as_slice() {
                [] => vec![],
                [_, ..] => remote.not_on_remote(&asked)?,
            };
            let missing = missing.into_iter().collect();
            Ok((id_len, asked.into_iter().collect(), missing))
        };
        Self(read())
    }

    /// No revision read, for `why`.
    pub fn unread(why: &str) -> Self {
        Self(Err(BoardStoreError::Unavailable(why.to_string())))
    }
}

impl RemoteRevisions for StatedRemoteRevisions {
    fn not_on_remote(&self, revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        let (_, asked, missing) = self.0.as_ref().map_err(Clone::clone)?;
        match revisions.iter().all(|revision| asked.contains(revision)) {
            true => {
                let held = |revision: &&String| missing.contains(*revision);
                Ok(revisions.iter().filter(held).cloned().collect())
            }
            false => Err(BoardStoreError::Unavailable(
                "a revision was not read before the write".into(),
            )),
        }
    }

    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError> {
        let read = self.0.as_ref().map(|(id_len, _, _)| *id_len);
        read.map_err(Clone::clone)
    }
}

#[cfg(test)]
#[path = "run_facts_tests.rs"]
mod tests;
