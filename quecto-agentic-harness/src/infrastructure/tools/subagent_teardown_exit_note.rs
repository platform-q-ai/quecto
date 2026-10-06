//! What a natural exit's note says about how the child ended (#2192), and
//! the exit status it reads for a child this harness held (#2260).
//!
//! The monitor's EOF can win the terminal claim once the supervisor has
//! reaped the process but before the reaper task has published what it
//! reaped. The supervisor recorded the exit in the same critical section
//! as the reap, so the note reads it there — and publishes it on the row's
//! exit signal first, so every other read of the ended row (`get_state`,
//! `get_messages`, the default report) agrees with the note.
use super::super::subagent_registry::{ExitSignal, SubagentEntry};
use super::{RegistryDelegatedAgents, TRANSCRIPT_STAYS_READABLE};

impl RegistryDelegatedAgents {
    /// How the target ended, for its natural-exit note, in the words every
    /// other view of its end uses: its exit status (what the supervisor
    /// reaped, for a child this harness held; `standing`, the status the
    /// row stood with before its compensation, for any other) and the
    /// crash record it left. `None` when nothing beyond the end is known.
    ///
    /// A reaped exit is published on the row before anything else, whether
    /// or not the note then has anything to say, and so before the note and
    /// the row's release to its joiners (#1953).
    pub(super) async fn end_detail(
        &self,
        target: Option<&SubagentEntry>,
        standing: Option<ExitSignal>,
        observation: &str,
    ) -> Option<String> {
        let target = target?;
        let reaped = publish_reaped_exit(target);
        let ended = self.ended.as_ref()?;
        let exit = match (target.owned_child, reaped) {
            (Some(_), Some(reaped)) => Some(reaped),
            (Some(_), None) => self.unrecorded_exit(target).await,
            (None, _) => standing,
        };
        let crash = ended
            .crash(
                &target.agent_uuid,
                target.origin,
                super::super::agent_cmd_ended::vouched_pid(target),
            )
            .await;
        let end = super::super::agent_cmd_ended::child_end_of(target, exit.as_ref(), crash);
        // Nothing observed and nothing left: the note's own wording says so.
        // How the end was observed stays in the note (#2192 review), as it
        // does when nothing else is known.
        let reason = match (end.kind(), &end.crash) {
            (crate::domain::agents::child_end::EndKind::Unknown, None) => return None,
            _ => format!(
                "{} ({})",
                end.reason(),
                crate::domain::agents::child_end::shown(observation, 64)
            ),
        };
        // The transcript is offered only when it can be read (#2192
        // review): a child this harness launched, whose store is this one's.
        match ended
            .has_transcript(&target.agent_uuid, target.origin)
            .await
        {
            true => Some(format!("{reason}. {TRANSCRIPT_STAYS_READABLE}")),
            false => Some(reason),
        }
    }

    /// The exit status of a child this harness held that the supervisor has
    /// no record of (no supervisor on the row, or its record gone from the
    /// retired ring): what the reaper publishes, waited for up to
    /// `exit_status_wait`.
    async fn unrecorded_exit(&self, target: &SubagentEntry) -> Option<ExitSignal> {
        // A row whose child is still held never reaches a natural exit's
        // note: its connection-level end defers to the reaper. This reads
        // the supervisor a second time, after `reaped_exit` found nothing;
        // that is harmless, since a slot's `reaped` never goes back to
        // false — a reap in between can only make the check pass, never
        // fail it spuriously — and it is compiled into debug builds only.
        debug_assert!(
            !target.holds_owned_child(),
            "an exit note for a child the supervisor still holds"
        );
        let tx = target.exit_signal_tx.as_ref()?;
        let mut published = tx.subscribe();
        let exit = tokio::time::timeout(self.exit_status_wait, published.wait_for(Option::is_some))
            .await
            .ok()
            .and_then(|seen| seen.ok().and_then(|exit| exit.clone()));
        match &exit {
            Some(_) => {}
            None => tracing::warn!(
                agent = %target.agent_uuid,
                wait = ?self.exit_status_wait,
                "exit note: the reaper had not published the child's exit status in time"
            ),
        }
        exit
    }
}

/// The exit the supervisor reaped `target`'s owned child with, published
/// on the row's exit signal when nothing is there yet (#2260 review round
/// 2). It is exactly the value the reaper task writes when it runs, so its
/// later `send_replace` changes nothing, and a publish the reaper made
/// first is left as it stands. `None` when the row names no owned child,
/// carries no supervisor, or the supervisor has no record of the reap.
fn publish_reaped_exit(target: &SubagentEntry) -> Option<ExitSignal> {
    let handle = target.owned_child?;
    let reaped = target
        .owned_child_supervisor
        .as_ref()?
        .reaped_exit(handle)?;
    let exit = super::super::spawn_reaper::exit_signal_from_exit(Some(reaped));
    if let Some(tx) = target.exit_signal_tx.as_ref() {
        tx.send_if_modified(|current| match current {
            None => {
                *current = Some(exit.clone());
                true
            }
            Some(_) => false,
        });
        debug_assert!(tx.borrow().is_some(), "the row's exit is published");
    }
    Some(exit)
}
