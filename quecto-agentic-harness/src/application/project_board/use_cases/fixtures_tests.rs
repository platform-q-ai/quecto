//! Fixtures of the project board's use case tests: a set clock, a board
//! with clients, and new tasks' fields.
use std::sync::{Arc, Mutex};

use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
use crate::application::project_board::ports::{
    BoardClock, BoardClockError, ProjectBoardStore, ReferenceStatus,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{Resolution, TaskKind, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

pub fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

pub fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// The refusal of task `id`, whose file the in-memory store set aside.
pub fn set_aside_task(id: &str) -> crate::application::project_board::dto::ProjectBoardError {
    crate::application::project_board::dto::ProjectBoardError::Quarantined {
        key: crate::application::project_board::ports::ArtifactKey::Task(slug(id)),
        path: format!("tasks/{id}.md"),
    }
}

pub fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

/// A clock reading a set time, with an optional outside reference; it
/// counts the pauses between attempts instead of waiting, and its
/// monotonic time moves on by `tick` each time it is read.
pub struct SetClock {
    pub now: Mutex<Timestamp>,
    pub reference: Option<Timestamp>,
    pub pauses: Mutex<Vec<usize>>,
    pub elapsed: Mutex<std::time::Duration>,
    pub tick: Mutex<std::time::Duration>,
}

impl SetClock {
    pub fn at(time: &str) -> Arc<Self> {
        Self::with_reference(time, None)
    }

    pub fn with_reference(time: &str, reference: Option<&str>) -> Arc<Self> {
        Self::reading(at(time), reference.map(at))
    }

    /// A clock reading `now`, with `reference`, on any day.
    pub fn reading(now: Timestamp, reference: Option<Timestamp>) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            reference,
            pauses: Mutex::new(Vec::new()),
            elapsed: Mutex::new(std::time::Duration::ZERO),
            tick: Mutex::new(std::time::Duration::ZERO),
        })
    }
}

impl BoardClock for SetClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        Ok(self.now.lock().unwrap().clone())
    }

    fn reference(&self) -> Option<Timestamp> {
        self.reference.clone()
    }

    fn reference_status(&self) -> ReferenceStatus {
        match self.reference {
            Some(_) => ReferenceStatus::Referenced,
            None => ReferenceStatus::NoReference("set without one".into()),
        }
    }

    fn pause(&self, attempt: usize) {
        self.pauses.lock().unwrap().push(attempt);
    }

    fn elapsed(&self) -> std::time::Duration {
        let mut elapsed = self.elapsed.lock().unwrap();
        let now = *elapsed;
        *elapsed += *self.tick.lock().unwrap();
        now
    }
}

/// A bootstrapped, empty board.
pub fn board() -> MemoryBoardStore {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    store
}

/// A new ready task.
pub fn new_task(id: &str) -> NewBoardTask {
    NewBoardTask {
        id: slug(id),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: String::new(),
        status: NewBoardTaskStatus::Ready,
        parent: None,
        depends_on: vec![],
    }
}

/// A new task's fields: ready, unclaimed, with no parts.
pub fn fields(id: &str) -> TaskFields {
    TaskFields {
        id: slug(id),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at("00:00:00"),
        updated: at("00:00:00"),
    }
}

/// `fields` as the board holds them: a done task is completed.
pub fn finished(fields: TaskFields) -> TaskFields {
    let resolution = match fields.status {
        TaskStatus::Done => Some(Resolution::Completed),
        _ => fields.resolution,
    };
    TaskFields {
        resolution,
        ..fields
    }
}

/// Task item `id` of a plan, after `depends_on`.
pub fn plan_item(
    id: &str,
    depends_on: &[&str],
) -> crate::domain::project_board::entities::plan::PlanItem {
    crate::domain::project_board::entities::plan::PlanItem {
        id: slug(id),
        title: format!("Item {id}"),
        criteria: vec![format!("{id} proved")],
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
    }
}

/// A team of one worker role.
pub fn plan_team() -> crate::domain::project_board::entities::team::Team {
    use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
    Team {
        roles: vec![Role {
            name: slug("worker"),
            model: "gpt-5.5".into(),
            effort: crate::domain::project_board::value_objects::vocabulary::Effort::High,
            limits: RoleLimits {
                members: 2,
                turns: None,
            },
        }],
        deadline_seconds: None,
        image: None,
    }
}

/// [`plan_team`] led by a coordinator role of one member on `model`.
pub fn coordinated_team(model: &str) -> crate::domain::project_board::entities::team::Team {
    use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
    let coordinator = Role {
        name: slug("coordinator"),
        model: model.into(),
        effort: crate::domain::project_board::value_objects::vocabulary::Effort::High,
        limits: RoleLimits {
            members: 1,
            turns: None,
        },
    };
    let workers = plan_team();
    Team {
        roles: std::iter::once(coordinator).chain(workers.roles).collect(),
        ..workers
    }
}

/// The refusal of a plan whose coordinator role's model names no
/// provider, as a start refuses it (S4c), in its words.
pub fn unqualified_coordinator() -> crate::application::project_board::dto::ProjectBoardError {
    crate::application::project_board::dto::ProjectBoardError::Brief {
        code: "coordinator_model",
        reason: "the coordinator role's model must name its provider, as provider/model: \
                 give it one"
            .into(),
    }
}

/// Version `version` of task `task`'s plan in `state`, proposed by Bob at
/// 09:00 with two task items (`api` after `schema`) and a team.
pub fn plan_of(
    task: &str,
    version: u32,
    state: crate::domain::project_board::value_objects::vocabulary::PlanState,
) -> crate::domain::project_board::entities::plan::Plan {
    use crate::domain::project_board::entities::plan::{Plan, PlanFields};
    use crate::domain::project_board::value_objects::vocabulary::PlanState;
    let accepted = match state {
        PlanState::Accepted | PlanState::Superseded => Some((person("Bob"), at("09:00:00"))),
        PlanState::Draft | PlanState::Discarded => None,
    };
    Plan::new(PlanFields {
        task: slug(task),
        version,
        state,
        approach: "Spike, then build.".into(),
        items: vec![plan_item("schema", &[]), plan_item("api", &["schema"])],
        team: Some(plan_team()),
        proposed_by: person("Bob"),
        edited_by: None,
        accepted,
        created: at("09:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap()
}

/// A board holding task `id` in `status` (written at 09:00) and `plans`.
pub fn holding_plans(
    id: &str,
    status: TaskStatus,
    plans: Vec<crate::domain::project_board::entities::plan::Plan>,
) -> MemoryBoardStore {
    use crate::application::project_board::ports::BoardArtifact;
    use crate::domain::project_board::entities::task::Task;
    let store = board();
    let head = store.read().unwrap().head;
    let task = Task::new(finished(TaskFields {
        status,
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields(id)
    }))
    .unwrap();
    let artifacts: Vec<BoardArtifact> = std::iter::once(BoardArtifact::Task(task))
        .chain(plans.into_iter().map(BoardArtifact::Plan))
        .collect();
    store.write(&head, &artifacts, "seed").unwrap();
    store
}

/// The board's remote as told (#2666): it holds every revision but those
/// it is `missing`, or cannot be read at all; it notes every ask.
#[derive(Default)]
pub struct Remote {
    pub missing: Vec<String>,
    pub down: bool,
    pub asked: Mutex<Vec<Vec<String>>>,
}

impl Remote {
    /// A remote holding every revision.
    pub fn holding_all() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A remote holding every revision but `missing`.
    pub fn missing(missing: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            missing: missing
                .iter()
                .map(|revision| (*revision).to_string())
                .collect(),
            ..Self::default()
        })
    }

    /// A remote that cannot be reached.
    pub fn down() -> Arc<Self> {
        Arc::new(Self {
            down: true,
            ..Self::default()
        })
    }

    /// Every ask, in order.
    pub fn asks(&self) -> Vec<Vec<String>> {
        self.asked.lock().unwrap().clone()
    }
}

impl crate::application::project_board::ports::RemoteRevisions for Remote {
    fn not_on_remote(
        &self,
        revisions: &[String],
    ) -> Result<Vec<String>, crate::application::project_board::ports::BoardStoreError> {
        self.asked.lock().unwrap().push(revisions.to_vec());
        match self.down {
            true => Err(
                crate::application::project_board::ports::BoardStoreError::Unavailable(
                    "the remote is down".into(),
                ),
            ),
            false => Ok(revisions
                .iter()
                .filter(|revision| self.missing.contains(revision))
                .cloned()
                .collect()),
        }
    }
}
