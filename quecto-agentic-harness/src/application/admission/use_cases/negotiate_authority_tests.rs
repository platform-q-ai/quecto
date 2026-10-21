use super::*;
use crate::application::admission::dto::negotiation::{NegotiationInputs, NegotiationPlan};
use crate::application::admission::ports::{AuthorityLaunch, LaunchState, LaunchedAuthority};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A launcher scripted per call: probe answers (unreachable once the
/// script runs out), launch outcomes and the launched broker's states
/// (running once that script runs out).
#[derive(Default)]
struct Scripted {
    probes: Mutex<VecDeque<Result<(), String>>>,
    probed: Mutex<Vec<PathBuf>>,
    launch_error: Option<String>,
    launches: Mutex<Vec<AuthorityLaunch>>,
    states: Arc<Mutex<VecDeque<LaunchState>>>,
    pauses: Mutex<Vec<u32>>,
}

impl Scripted {
    fn probing(probes: Vec<Result<(), &str>>) -> Self {
        Self {
            probes: Mutex::new(
                probes
                    .into_iter()
                    .map(|probe| probe.map_err(str::to_string))
                    .collect(),
            ),
            ..Self::default()
        }
    }

    fn with_states(self, states: Vec<LaunchState>) -> Self {
        *self.states.lock().unwrap() = states.into();
        self
    }

    fn launches(&self) -> usize {
        self.launches.lock().unwrap().len()
    }

    fn pauses(&self) -> Vec<u32> {
        self.pauses.lock().unwrap().clone()
    }
}

struct ScriptedBroker(Arc<Mutex<VecDeque<LaunchState>>>);

impl LaunchedAuthority for ScriptedBroker {
    fn state(&mut self) -> LaunchState {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(LaunchState::Running)
    }
}

impl AuthorityLauncher for Scripted {
    fn probe(&self, directory: &Path) -> Result<(), String> {
        self.probed.lock().unwrap().push(directory.to_path_buf());
        self.probes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err("connection refused".into()))
    }

    fn launch(&self, launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        self.launches.lock().unwrap().push(launch.clone());
        match &self.launch_error {
            Some(error) => Err(error.clone()),
            None => Ok(Box::new(ScriptedBroker(self.states.clone()))),
        }
    }

    fn pause(&self, attempt: u32) {
        self.pauses.lock().unwrap().push(attempt);
    }
}

fn use_case(launcher: &Arc<Scripted>) -> NegotiateAuthority {
    NegotiateAuthority::new(launcher.clone())
}

fn launch() -> AuthorityLaunch {
    AuthorityLaunch {
        directory: PathBuf::from("/x/admission"),
        config: PathBuf::from("/x/config.json"),
        base_dir: PathBuf::from("/x"),
    }
}

#[test]
fn a_child_context_wins_even_when_a_section_is_configured() {
    // #2023: a child with its own (possibly diverging or null) config still
    // inherits the parent's authority; the context always wins.
    let plan = use_case(&Arc::default()).execute(NegotiationInputs {
        configured_directory: Some(PathBuf::from("/x/admission")),
        inherited_context: Some(PathBuf::from("/tmp/ctx")),
    });
    assert_eq!(
        plan,
        NegotiationPlan::Child {
            context: PathBuf::from("/tmp/ctx")
        }
    );
}

#[test]
fn a_configured_root_registers_at_its_directory() {
    let plan = use_case(&Arc::default()).execute(NegotiationInputs {
        configured_directory: Some(PathBuf::from("/x/admission")),
        inherited_context: None,
    });
    assert_eq!(
        plan,
        NegotiationPlan::Root {
            directory: PathBuf::from("/x/admission")
        }
    );
}

#[test]
fn nothing_configured_is_disabled() {
    let plan = use_case(&Arc::default()).execute(NegotiationInputs {
        configured_directory: None,
        inherited_context: None,
    });
    assert_eq!(plan, NegotiationPlan::Disabled);
}

#[test]
fn a_reachable_authority_is_joined_without_a_launch() {
    let launcher = Arc::new(Scripted::probing(vec![Ok(())]));
    let outcome = use_case(&launcher).ensure_running(&launch());
    assert_eq!(outcome, Ok(AuthorityAvailability::AlreadyRunning));
    assert_eq!(launcher.launches(), 0);
    assert!(launcher.pauses().is_empty());
    assert_eq!(
        *launcher.probed.lock().unwrap(),
        vec![PathBuf::from("/x/admission")]
    );
}

#[test]
fn an_unreachable_authority_is_started_and_probed_with_backoff() {
    let launcher = Arc::new(Scripted::probing(vec![
        Err("refused"),
        Err("refused"),
        Ok(()),
    ]));
    let outcome = use_case(&launcher).ensure_running(&launch());
    assert_eq!(outcome, Ok(AuthorityAvailability::StartedOnDemand));
    assert_eq!(*launcher.launches.lock().unwrap(), vec![launch()]);
    assert_eq!(launcher.pauses(), vec![0, 1]);
}

#[test]
fn a_lost_race_waits_for_the_winner_without_starting_another() {
    // The loser exits Busy; the winner answers on the next probe.
    let launcher = Arc::new(
        Scripted::probing(vec![Err("refused"), Ok(())])
            .with_states(vec![LaunchState::LostRace, LaunchState::LostRace]),
    );
    let outcome = use_case(&launcher).ensure_running(&launch());
    assert_eq!(outcome, Ok(AuthorityAvailability::StartedOnDemand));
    assert_eq!(launcher.launches(), 1, "the winner was given its step");
}

#[test]
fn a_lost_race_to_a_broker_that_never_answers_starts_another() {
    // The lock holder was stopping when this start lost to it: once it is
    // still unreachable a step later, a fresh broker is started.
    let launcher = Arc::new(
        Scripted::probing(vec![Err("refused"), Err("refused"), Err("refused"), Ok(())])
            .with_states(vec![LaunchState::LostRace, LaunchState::LostRace]),
    );
    let outcome = use_case(&launcher).ensure_running(&launch());
    assert_eq!(outcome, Ok(AuthorityAvailability::StartedOnDemand));
    assert_eq!(launcher.launches(), 2);
}

#[test]
fn a_broker_that_fails_stops_the_wait_with_its_reason() {
    let launcher = Arc::new(
        Scripted::probing(vec![]).with_states(vec![LaunchState::Failed {
            reason: "exited with status 1; see /x/admission/broker.log".into(),
        }]),
    );
    let error = use_case(&launcher).ensure_running(&launch()).unwrap_err();
    assert!(error.contains("see /x/admission/broker.log"), "{error}");
    assert_eq!(launcher.pauses(), vec![0], "no further probes");
}

#[test]
fn a_launch_that_cannot_start_is_an_error() {
    let launcher = Arc::new(Scripted {
        launch_error: Some("not the quecto binary".into()),
        ..Scripted::default()
    });
    let error = use_case(&launcher).ensure_running(&launch()).unwrap_err();
    assert!(error.contains("not the quecto binary"), "{error}");
    assert!(launcher.pauses().is_empty());
}

#[test]
fn a_broker_that_never_answers_is_given_up_after_the_bound() {
    let launcher = Arc::new(Scripted::probing(vec![]));
    let error = use_case(&launcher).ensure_running(&launch()).unwrap_err();
    assert_eq!(launcher.pauses(), (0..ENSURE_ATTEMPTS).collect::<Vec<_>>());
    assert_eq!(launcher.launches(), 1);
    assert!(error.contains("/x/admission"), "{error}");
    assert!(error.contains("connection refused"), "{error}");
}
