//! Final-member environment cleanup (#1369, #1924, #1939).
//!
//! Membership removal may mint the exclusive final cleanup claim atomically
//! in the domain registry. This use case owns the surrounding application
//! transaction: the one post-mortem inspect per dead member, the retention
//! decision (the domain's), the coordinator-loss record, and the retained
//! `kill` / `cleanup` runs. Infrastructure supplies only the script
//! executions and the observation of a hosted swarm run through the
//! capability's ports.
use std::sync::Arc;

use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, KillClaim,
};
use crate::domain::environments::services::environment_retention::{
    CompletionDelivery, MemberFinalizeMode, SwarmRunObservation, ends_plain_environment,
    judges_delivery, loss_reason, retains_environment, retention_reason, undelivered_reason,
    unrecorded_loss_reason,
};

use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::application::environments::ports::{
    hosted_swarm::CompletionDeliveryCheck, hosted_swarm::EndedSwarmBoards,
    hosted_swarm::HostedSwarmBoard, hosted_swarm::HostedSwarmRunObservation,
    process::EnvironmentProcessCommands,
};

pub struct FinalizeEnvironmentMember {
    registry: EnvironmentRegistry,
    commands: Arc<dyn EnvironmentProcessCommands>,
    hosted: Arc<dyn HostedSwarmRunObservation>,
    /// Where a created swarm run's board is read and kept as its member
    /// leaves, before any teardown; `None`: none is kept.
    ended: Option<(Arc<dyn HostedSwarmBoard>, Arc<dyn EndedSwarmBoards>)>,
    /// Where a succeeded run's completion went, judged on its owner's kill
    /// (#2666 round 2); `None`: not judged.
    delivery: Option<Arc<dyn CompletionDeliveryCheck>>,
}

impl FinalizeEnvironmentMember {
    pub fn new(
        registry: EnvironmentRegistry,
        commands: Arc<dyn EnvironmentProcessCommands>,
        hosted: Arc<dyn HostedSwarmRunObservation>,
    ) -> Self {
        Self {
            registry,
            commands,
            hosted,
            ended: None,
            delivery: None,
        }
    }

    /// This use case, keeping the environment of a succeeded run its owner
    /// kills while `check` finds its completion commit not on the board's
    /// remote, or cannot tell (#2666 round 2): the work survives in its
    /// workspace, to be pushed from there.
    #[must_use]
    pub fn checking_delivery(self, check: Arc<dyn CompletionDeliveryCheck>) -> Self {
        Self {
            delivery: Some(check),
            ..self
        }
    }

    /// This use case, keeping in `kept` the board of a created swarm run
    /// that has ended, read through `boards`, as the member whose exit
    /// empties its environment leaves, under the session and that member's
    /// label: before the retention decision and any teardown, so the board
    /// outlives the box (#2481 S4c follow-up).
    #[must_use]
    pub fn keeping_ended_boards(
        self,
        boards: Arc<dyn HostedSwarmBoard>,
        kept: Arc<dyn EndedSwarmBoards>,
    ) -> Self {
        Self {
            ended: Some((boards, kept)),
            ..self
        }
    }

    /// Remove `agent_uuid` from `env_ref`; when that empties a running
    /// environment, run its final teardown exactly once under the claim the
    /// removal minted. `entry_cleanup_plan` is the member's own retained
    /// cleanup (creator's record) used only when the environment retained no
    /// cleanup argv of its own.
    pub async fn finalize_member(
        &self,
        env_ref: &str,
        agent_uuid: &str,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        self.finalize_labelled_member(env_ref, agent_uuid, None, entry_cleanup_plan, mode)
            .await;
    }

    /// [`Self::finalize_member`] for the member spawned as `label`, when it
    /// has one: the name an ended run's board is kept under.
    pub async fn finalize_labelled_member(
        &self,
        env_ref: &str,
        agent_uuid: &str,
        label: Option<&str>,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        if mode == MemberFinalizeMode::Exit {
            self.inspect_once(env_ref, agent_uuid).await;
        }

        let Ok(Some(claim)) = self.registry.remove_member(env_ref, agent_uuid) else {
            return;
        };
        let Some(record) = self.registry.get(env_ref) else {
            self.registry.complete_kill(claim);
            return;
        };

        let observed = if mode.inspectable_end() {
            self.hosted.observe_hosted_swarm_run(&record).await
        } else {
            SwarmRunObservation::NoStore
        };
        // An ended run's board is kept before anything can tear it down:
        // one still running or paused may yet change, and is not.
        let board = match (&observed, label) {
            (SwarmRunObservation::Run(hosted), Some(label))
                if hosted.created() && hosted.status.terminal() =>
            {
                self.keep_board(&record, label).await
            }
            (
                SwarmRunObservation::Run(_)
                | SwarmRunObservation::NoStore
                | SwarmRunObservation::NoStoreUnverified
                | SwarmRunObservation::Unreadable(_),
                _,
            ) => None,
        };
        if retains_environment(mode, &observed) {
            // `None`: the owner closed the run between the observation and
            // the loss record (#2070) — the swarm is over, the kill runs.
            if let Some(reason) = self.retention(&record, mode, &observed).await {
                self.registry.retain(claim, &reason);
                return;
            }
        }

        // #2666 round 2: a succeeded run its owner kills keeps its box
        // while its completion commit is not on the board's remote, or
        // cannot be told to be: the work survives, to be pushed from it.
        if let Some(reason) = self
            .undelivered(mode, &observed, (&record, label), board.as_ref())
            .await
        {
            self.registry.retain(claim, &reason);
            return;
        }

        // #2206: on the owner's word a plain container child's box has
        // nothing to resume — it goes for good, record included.
        if ends_plain_environment(mode, &observed) {
            let plan = Self::cleanup_plan(&record, entry_cleanup_plan.clone());
            if let Some((environment_id, argv)) = plan {
                self.end_for_good(claim, &record, &environment_id, &argv)
                    .await;
                return;
            }
        }

        if mode.launch_rollback() || record.retained_kill_argv.is_empty() {
            self.cleanup(claim, &record, entry_cleanup_plan, mode).await;
            return;
        }

        match self
            .commands
            .run_retained_kill(&record.environment_id, &record.retained_kill_argv)
            .await
        {
            Ok(()) => self.registry.complete_kill(claim),
            Err(e) => self.registry.fail_kill(claim, &e),
        }
    }

    /// Keeps the board `record` hosts under this session and `label`, when
    /// this use case keeps boards: best effort, as a board that cannot be
    /// read is logged and the teardown goes on.
    /// It returns the board read, when one was.
    async fn keep_board(
        &self,
        record: &EnvironmentRecord,
        label: &str,
    ) -> Option<SwarmBoardSnapshot> {
        let (boards, kept) = self.ended.as_ref()?;
        match boards.board(record).await {
            Ok(Some(board)) => {
                kept.keep(self.registry.session(), label, board.clone());
                Some(board)
            }
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    environment = %record.environment_ref,
                    %error,
                    "the ended swarm's board could not be kept"
                );
                None
            }
        }
    }

    /// Why the environment of `observed` is kept for its run's completion
    /// (#2666 round 2), when the teardown judges it and finds it not on
    /// the board's remote, or cannot tell.
    async fn undelivered(
        &self,
        mode: MemberFinalizeMode,
        observed: &SwarmRunObservation,
        (record, label): (&EnvironmentRecord, Option<&str>),
        board: Option<&SwarmBoardSnapshot>,
    ) -> Option<String> {
        let (Some(check), Some(label)) = (&self.delivery, label) else {
            return None;
        };
        let delivery = match judges_delivery(mode, observed) {
            true => check.delivery(self.registry.session(), label, board).await,
            false => CompletionDelivery::NotJudged,
        };
        let workspace = record.workspace_path.display().to_string();
        undelivered_reason(&delivery, &record.environment_ref, &workspace)
    }

    /// The cleanup that ends `record`: the environment's own retained
    /// cleanup argv, else the member's plan; `None` when neither names one.
    fn cleanup_plan(
        record: &EnvironmentRecord,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
    ) -> Option<(String, Vec<String>)> {
        match record.retained_cleanup_argv.first() {
            Some(_program) => Some((
                record.environment_id.clone(),
                record.retained_cleanup_argv.clone(),
            )),
            None => {
                entry_cleanup_plan.filter(|(_, argv)| matches!(argv.as_slice(), [_program, ..]))
            }
        }
    }

    /// End a plain container child's environment for good (#2206): the
    /// retained `cleanup` removes the container and its state directory,
    /// and only once it reported success is the record forgotten, as a
    /// launch rollback does. A failed cleanup leaves the record
    /// `cleanup-failed` with the script's account, so `container kill`
    /// retries it and `ls` still shows what is left.
    async fn end_for_good(
        &self,
        claim: KillClaim,
        record: &EnvironmentRecord,
        environment_id: &str,
        argv: &[String],
    ) {
        debug_assert!(!argv.is_empty(), "only a named cleanup ends a box for good");
        match self
            .commands
            .run_retained_cleanup(environment_id, argv)
            .await
        {
            Ok(()) => {
                self.registry.complete_kill(claim);
                self.registry.remove(&record.environment_ref);
            }
            Err(error) => self.registry.fail_kill(claim, &error),
        }
    }

    /// The retained `cleanup` contract: the environment's own cleanup argv,
    /// else the member's plan; then stopped, and for the launch that created
    /// the environment the record is discarded entirely. Best effort: a
    /// rollback has nothing usable to keep, so its outcome only is logged.
    async fn cleanup(
        &self,
        claim: KillClaim,
        record: &EnvironmentRecord,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        if let Some((environment_id, argv)) = Self::cleanup_plan(record, entry_cleanup_plan) {
            // The adapter reports a failure itself; best effort by contract.
            let _ = self
                .commands
                .run_retained_cleanup(&environment_id, &argv)
                .await;
        }
        self.registry.complete_kill(claim);
        if mode == MemberFinalizeMode::LaunchRollbackOwned {
            self.registry.remove(&record.environment_ref);
        }
    }

    /// The reason the retained kill is withheld (#1924), or `None` when it
    /// must run after all. On the member's own exit a created run that has
    /// not ended is recorded as a coordinator loss through the port (one
    /// store operation decides); a supervisor's kill only records what it
    /// found. A store that refuses the loss record still leaves the
    /// environment retained: losing the box is never the safer outcome. The
    /// one exception is a run its owner closed in between (#2070).
    async fn retention(
        &self,
        record: &EnvironmentRecord,
        mode: MemberFinalizeMode,
        observed: &SwarmRunObservation,
    ) -> Option<String> {
        Some(match observed {
            SwarmRunObservation::Run(hosted) if mode == MemberFinalizeMode::Exit => {
                match self.hosted.record_lost_coordinator(record, hosted).await {
                    Ok(loss) if !loss.lost && !loss.run.keeps_environment() => return None,
                    Ok(loss) => loss_reason(&hosted.coordinator, &loss),
                    Err(error) => unrecorded_loss_reason(hosted, &error),
                }
            }
            observed => retention_reason(mode, observed),
        })
    }

    async fn inspect_once(&self, env_ref: &str, agent_uuid: &str) {
        let Some(record) = self.registry.get(env_ref) else {
            return;
        };
        if record.retained_inspect_argv.is_empty() {
            return;
        }
        let Some(claim) = self.registry.begin_inspect(env_ref, agent_uuid) else {
            return;
        };
        match self
            .commands
            .run_retained_inspect(&record.environment_id, &record.retained_inspect_argv)
            .await
        {
            Ok(metadata) => self.registry.record_inspect_success(claim, metadata),
            Err(e) => self.registry.record_inspect_failure(claim, &e),
        }
    }
}
