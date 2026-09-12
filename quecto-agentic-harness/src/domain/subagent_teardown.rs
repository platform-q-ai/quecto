//! Pure teardown policy for delegated agents (#1934, epic #1929).
//!
//! Two distinct operations share these invariants:
//!
//! - **self shutdown** asks a harness to end itself and its directly owned
//!   subtree, for an allowlisted [`ShutdownReason`];
//! - **selected termination** asks a harness to resolve the next direct edge
//!   toward one selected descendant, identified by uuid *and* launch
//!   generation, within a bounded [`RoutingDepth`]. The harness that directly
//!   owns the target shuts that child down; every intermediate stays alive.
//!
//! Everything here is deterministic value/policy code: no sockets, processes,
//! clocks or persistence. Effects belong to application ports.
use std::collections::HashSet;
use std::fmt;

use super::ids::AgentUuid;

/// Monotonic launch generation of a delegated agent. A uuid that is re-used
/// by a later launch carries a different generation, so a stale command aimed
/// at the earlier launch can never reach the newer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaunchGeneration(u64);

impl LaunchGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Durable identity of one launch of a delegated agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DelegatedAgentIdentity {
    pub uuid: AgentUuid,
    pub generation: LaunchGeneration,
}

impl DelegatedAgentIdentity {
    pub fn new(uuid: impl Into<AgentUuid>, generation: LaunchGeneration) -> Self {
        Self {
            uuid: uuid.into(),
            generation,
        }
    }
}

/// Remaining hops a selected-termination command may still travel.
///
/// One hop is the direct edge from the receiving harness to a direct child.
/// Zero is meaningless (the command could go nowhere) and anything above
/// [`RoutingDepth::MAX_HOPS`] is rejected outright so a malformed or hostile
/// request cannot fan out through an unbounded delegation tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoutingDepth(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDepthError {
    Zero,
    ExceedsMaximum { requested: u32, maximum: u32 },
}

impl fmt::Display for RoutingDepthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => f.write_str("remaining_depth must be at least 1"),
            Self::ExceedsMaximum { requested, maximum } => {
                write!(f, "remaining_depth {requested} exceeds maximum {maximum}")
            }
        }
    }
}

impl RoutingDepth {
    /// Deepest delegation tree a single command may traverse. Kept equal to
    /// the inspection walker's `MAX_INSPECTION_ROUTE_DEPTH`
    /// (`infrastructure/tools/subagent_routing.rs`) so both bounds agree;
    /// that walker should consume [`LineageSnapshot`] in a later slice
    /// instead of re-walking registry `parent_id` chains.
    pub const MAX_HOPS: u32 = 32;

    pub const fn new(hops: u32) -> Result<Self, RoutingDepthError> {
        if hops == 0 {
            return Err(RoutingDepthError::Zero);
        }
        if hops > Self::MAX_HOPS {
            return Err(RoutingDepthError::ExceedsMaximum {
                requested: hops,
                maximum: Self::MAX_HOPS,
            });
        }
        Ok(Self(hops))
    }

    pub const fn hops(self) -> u32 {
        self.0
    }

    /// Depth left after forwarding across one edge; `None` when this hop was
    /// the last one the command was allowed to take.
    pub const fn after_forward(self) -> Option<Self> {
        match self.0 {
            0 | 1 => None,
            hops => Some(Self(hops - 1)),
        }
    }
}

/// Closed vocabulary of why a harness is asked to shut itself down. Unknown
/// reasons are rejected at the edge; there is no free-text variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShutdownReason {
    /// The launching parent is shutting down its whole subtree.
    ParentShutdown,
    /// An ancestor selected this agent for termination (`terminate_delegated_agent`).
    SelectedTermination,
    /// The authenticated launch-bound parent connection closed.
    ParentConnectionLost,
    /// A launched harness's parent never bound its control connection
    /// within the bind deadline: the launcher is presumed gone.
    ParentNeverBound,
    /// SIGTERM/SIGINT or equivalent delivered to the harness process.
    TerminationSignal,
    /// An operator asked for the roster to be torn down (delete-all, exit).
    OperatorRequest,
}

impl ShutdownReason {
    pub const ALL: [ShutdownReason; 6] = [
        Self::ParentShutdown,
        Self::SelectedTermination,
        Self::ParentConnectionLost,
        Self::ParentNeverBound,
        Self::TerminationSignal,
        Self::OperatorRequest,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParentShutdown => "parent_shutdown",
            Self::SelectedTermination => "selected_termination",
            Self::ParentConnectionLost => "parent_connection_lost",
            Self::ParentNeverBound => "parent_never_bound",
            Self::TerminationSignal => "termination_signal",
            Self::OperatorRequest => "operator_request",
        }
    }

    /// Affirmative allowlist parse: only the exact canonical spellings map.
    pub fn parse(raw: &str) -> Result<Self, UnknownShutdownReason> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.as_str() == raw)
            .ok_or_else(|| UnknownShutdownReason(raw.to_owned()))
    }
}

impl fmt::Display for ShutdownReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownShutdownReason(pub String);

impl fmt::Display for UnknownShutdownReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown shutdown reason {:?}", self.0)
    }
}

/// Lifecycle of the receiving harness with respect to teardown. New prompt
/// and spawn work is only accepted while `Accepting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HarnessLifecycleState {
    Accepting,
    /// Shutdown admitted: no new prompt/spawn work, but the harness lives on
    /// until execution finishes (or the admission is released).
    Frozen,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleTransitionError {
    pub from: HarnessLifecycleState,
    pub attempted: &'static str,
}

impl fmt::Display for LifecycleTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot {} from {:?}", self.attempted, self.from)
    }
}

impl HarnessLifecycleState {
    pub const fn accepts_new_work(self) -> bool {
        matches!(self, Self::Accepting)
    }

    pub const fn freeze(self) -> Result<Self, LifecycleTransitionError> {
        match self {
            Self::Accepting | Self::Frozen => Ok(Self::Frozen),
            Self::Terminated => Err(LifecycleTransitionError {
                from: self,
                attempted: "freeze",
            }),
        }
    }

    /// Lift an admission that never executed (its ACK could not be written).
    pub const fn thaw(self) -> Result<Self, LifecycleTransitionError> {
        match self {
            Self::Frozen | Self::Accepting => Ok(Self::Accepting),
            Self::Terminated => Err(LifecycleTransitionError {
                from: self,
                attempted: "thaw",
            }),
        }
    }

    /// Termination is only legal from a frozen harness: work must have been
    /// fenced off before the subtree is torn down.
    pub const fn terminate(self) -> Result<Self, LifecycleTransitionError> {
        match self {
            Self::Frozen | Self::Terminated => Ok(Self::Terminated),
            Self::Accepting => Err(LifecycleTransitionError {
                from: self,
                attempted: "terminate",
            }),
        }
    }
}

/// One delegated agent this harness knows about, and who launched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageRecord {
    pub identity: DelegatedAgentIdentity,
    pub parent: AgentUuid,
}

/// The receiving harness's view of its delegation subtree: every record whose
/// `parent` is `owner` is a direct child; deeper records were reported upward.
///
/// The snapshot must hold **current-generation records only**: one record per
/// live uuid. A uuid listed twice — even across generations — is
/// [`TerminationRouteError::AmbiguousLineage`], because the walk cannot tell
/// which edge is meant. Retiring a superseded generation is the lifecycle
/// repository's job, not the router's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageSnapshot {
    pub owner: AgentUuid,
    pub records: Vec<LineageRecord>,
}

impl LineageSnapshot {
    /// Records parented by the owner. A record claiming the owner's own uuid
    /// is a corrupt snapshot, never a child, and is skipped.
    pub fn direct_children(&self) -> impl Iterator<Item = &DelegatedAgentIdentity> {
        self.records
            .iter()
            .filter(|record| record.parent == self.owner && record.identity.uuid != self.owner)
            .map(|record| &record.identity)
    }

    /// Exactly one record for `uuid`, or `None` when unknown. Two records
    /// claiming one uuid make every edge through it unaffirmable.
    fn unique_record(
        &self,
        uuid: &AgentUuid,
    ) -> Result<Option<&LineageRecord>, TerminationRouteError> {
        let mut matches = self
            .records
            .iter()
            .filter(|record| &record.identity.uuid == uuid);
        let first = matches.next();
        if first.is_some() && matches.next().is_some() {
            return Err(TerminationRouteError::AmbiguousLineage(uuid.clone()));
        }
        Ok(first)
    }
}

/// The single edge a receiving harness resolves for a selected termination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationRoute {
    /// The target is a direct child: invoke self shutdown on it.
    ShutdownDirectChild(DelegatedAgentIdentity),
    /// The target lives deeper: forward with one hop consumed. The forwarding
    /// harness stays alive.
    ForwardToDirectChild {
        via: DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationRouteError {
    /// `terminate_delegated_agent` never targets the receiver itself; that is
    /// what `shutdown` is for.
    TargetIsSelf,
    UnknownTarget(AgentUuid),
    /// More than one record claims a uuid on the walk (target or
    /// intermediate): the snapshot cannot say which edge is meant, so none
    /// is taken.
    AmbiguousLineage(AgentUuid),
    StaleGeneration {
        target: AgentUuid,
        requested: LaunchGeneration,
        current: LaunchGeneration,
    },
    /// The lineage walk revisited a node or left the subtree: no edge is safe.
    LineageCycle(AgentUuid),
    /// The target is reachable but not within the remaining hop budget.
    DepthExhausted {
        target: AgentUuid,
        remaining: RoutingDepth,
    },
}

impl fmt::Display for TerminationRouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetIsSelf => f.write_str("target is the receiving harness; use shutdown"),
            Self::UnknownTarget(uuid) => write!(f, "unknown delegated agent {uuid}"),
            Self::AmbiguousLineage(uuid) => write!(f, "ambiguous lineage at {uuid}"),
            Self::StaleGeneration {
                target,
                requested,
                current,
            } => write!(
                f,
                "stale generation {} for {target} (current {})",
                requested.get(),
                current.get()
            ),
            Self::LineageCycle(uuid) => write!(f, "lineage cycle at {uuid}"),
            Self::DepthExhausted { target, remaining } => write!(
                f,
                "target {target} not reachable within {} remaining hop(s)",
                remaining.hops()
            ),
        }
    }
}

/// Resolve the next direct edge toward `target`, or refuse with no effect.
///
/// Rejections are affirmative: the target must be known, its generation must
/// match exactly, the parent walk must terminate at `owner` without revisiting
/// a node, and the number of edges must fit in `remaining_depth`.
pub fn resolve_termination_route(
    snapshot: &LineageSnapshot,
    target: &DelegatedAgentIdentity,
    remaining_depth: RoutingDepth,
) -> Result<TerminationRoute, TerminationRouteError> {
    if target.uuid == snapshot.owner {
        return Err(TerminationRouteError::TargetIsSelf);
    }
    let record = snapshot
        .unique_record(&target.uuid)?
        .ok_or_else(|| TerminationRouteError::UnknownTarget(target.uuid.clone()))?;
    if record.identity.generation != target.generation {
        return Err(TerminationRouteError::StaleGeneration {
            target: target.uuid.clone(),
            requested: target.generation,
            current: record.identity.generation,
        });
    }
    let (via, edges) = direct_child_toward(snapshot, record)?;
    // The walk only returns a record parented by the owner; anything else is
    // a broken snapshot and is refused rather than routed.
    let via_is_direct = snapshot
        .unique_record(&via.uuid)?
        .is_some_and(|record| record.parent == snapshot.owner);
    if !via_is_direct || edges == 0 {
        return Err(TerminationRouteError::LineageCycle(via.uuid.clone()));
    }
    if edges == 1 {
        if via.uuid != target.uuid {
            return Err(TerminationRouteError::LineageCycle(via.uuid));
        }
        return Ok(TerminationRoute::ShutdownDirectChild(via));
    }
    // The whole remaining path must fit the budget *now*: an over-depth route
    // is refused at the first hop instead of consuming edges toward a target
    // it can never reach.
    match remaining_depth.after_forward() {
        Some(next) if remaining_depth.hops() >= edges => {
            Ok(TerminationRoute::ForwardToDirectChild {
                via,
                remaining_depth: next,
            })
        }
        _ => Err(TerminationRouteError::DepthExhausted {
            target: target.uuid.clone(),
            remaining: remaining_depth,
        }),
    }
}

/// Walk parent pointers from `record` up to a direct child of the owner,
/// returning that child and the number of edges between owner and target.
fn direct_child_toward(
    snapshot: &LineageSnapshot,
    record: &LineageRecord,
) -> Result<(DelegatedAgentIdentity, u32), TerminationRouteError> {
    let mut visited: HashSet<&AgentUuid> = HashSet::with_capacity(snapshot.records.len());
    let mut current = record;
    loop {
        if !visited.insert(&current.identity.uuid) {
            return Err(TerminationRouteError::LineageCycle(
                current.identity.uuid.clone(),
            ));
        }
        if current.parent == snapshot.owner {
            let edges = u32::try_from(visited.len()).unwrap_or(u32::MAX);
            return Ok((current.identity.clone(), edges));
        }
        // Every node on the walk must be unique; a duplicated intermediate
        // would otherwise route down whichever copy happened to be listed
        // first.
        current = snapshot.unique_record(&current.parent)?.ok_or_else(|| {
            // A parent this harness has never seen is not a route it can
            // affirm; treat it like a broken lineage rather than guessing.
            TerminationRouteError::LineageCycle(current.parent.clone())
        })?;
    }
}

#[cfg(test)]
#[path = "subagent_teardown_tests.rs"]
mod tests;
