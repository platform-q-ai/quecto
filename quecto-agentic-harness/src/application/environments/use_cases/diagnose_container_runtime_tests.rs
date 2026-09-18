use std::sync::{Arc, Mutex};

use super::DiagnoseContainerRuntime;
use crate::application::environments::dto::{
    CheckStatus, ContainerRuntimeTarget, DiagnosableContainerConfig, DiagnoseContainerRuntimeError,
    PreflightCheck,
};
use crate::application::environments::ports::{ContainerConfigLookup, ContainerRuntimePreflight};

struct FixedLookup(Result<DiagnosableContainerConfig, String>);

impl ContainerConfigLookup for FixedLookup {
    fn lookup(&self, _: &ContainerRuntimeTarget) -> Result<DiagnosableContainerConfig, String> {
        self.0.clone()
    }
}

struct RecordingPreflight {
    answer: Result<Vec<PreflightCheck>, String>,
    seen: Mutex<Vec<DiagnosableContainerConfig>>,
}

impl ContainerRuntimePreflight for RecordingPreflight {
    fn preflight(
        &self,
        config: &DiagnosableContainerConfig,
    ) -> Result<Vec<PreflightCheck>, String> {
        self.seen.lock().unwrap().push(config.clone());
        self.answer.clone()
    }
}

fn config(create: &[&str]) -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "official".into(),
        create: create.iter().map(|s| s.to_string()).collect(),
        diagnostics: vec!["overlay note".into()],
    }
}

fn check(name: &str, status: CheckStatus) -> PreflightCheck {
    PreflightCheck {
        name: name.into(),
        status,
        detail: format!("{name} detail"),
        remedy: String::new(),
    }
}

fn use_case(
    lookup: Result<DiagnosableContainerConfig, String>,
    answer: Result<Vec<PreflightCheck>, String>,
) -> (DiagnoseContainerRuntime, Arc<RecordingPreflight>) {
    let preflight = Arc::new(RecordingPreflight {
        answer,
        seen: Mutex::new(Vec::new()),
    });
    (
        DiagnoseContainerRuntime::new(Arc::new(FixedLookup(lookup)), preflight.clone()),
        preflight,
    )
}

#[test]
fn a_diagnosis_carries_the_config_its_checks_and_the_layer_diagnostics() {
    let checks = vec![
        check("runtime-cli", CheckStatus::Passed),
        check("gh", CheckStatus::Warned),
        check("image", CheckStatus::Failed),
    ];
    let (doctor, preflight) = use_case(
        Ok(config(&["/bin/create", "--repo", "r"])),
        Ok(checks.clone()),
    );
    let diagnosis = doctor.execute(&ContainerRuntimeTarget::default()).unwrap();
    assert_eq!(diagnosis.config, "official");
    assert_eq!(diagnosis.create, ["/bin/create", "--repo", "r"]);
    assert_eq!(diagnosis.checks, checks);
    assert_eq!(diagnosis.diagnostics, ["overlay note"]);
    assert!(!diagnosis.healthy());
    assert_eq!(diagnosis.failed(), 1);
    assert_eq!(
        preflight.seen.lock().unwrap().as_slice(),
        [config(&["/bin/create", "--repo", "r"])]
    );
}

#[test]
fn warnings_alone_leave_a_diagnosis_healthy() {
    let (doctor, _) = use_case(
        Ok(config(&["/bin/create"])),
        Ok(vec![
            check("gh", CheckStatus::Warned),
            check("image", CheckStatus::Passed),
        ]),
    );
    let diagnosis = doctor.execute(&ContainerRuntimeTarget::default()).unwrap();
    assert!(diagnosis.healthy());
    assert_eq!(diagnosis.failed(), 0);
}

#[test]
fn an_unresolvable_target_is_the_selections_own_account_and_runs_no_preflight() {
    let (doctor, preflight) = use_case(Err("unknown container config 'nope'".into()), Ok(vec![]));
    let error = doctor
        .execute(&ContainerRuntimeTarget {
            name: Some("nope".into()),
        })
        .unwrap_err();
    assert_eq!(
        error,
        DiagnoseContainerRuntimeError::ConfigUnavailable("unknown container config 'nope'".into())
    );
    assert_eq!(error.to_string(), "unknown container config 'nope'");
    assert!(preflight.seen.lock().unwrap().is_empty());
}

#[test]
fn a_script_that_cannot_answer_names_the_config() {
    let (doctor, _) = use_case(
        Ok(config(&["/bin/create"])),
        Err("create script /bin/create does not support --preflight-only".into()),
    );
    let error = doctor
        .execute(&ContainerRuntimeTarget::default())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "container config 'official': create script /bin/create does not support --preflight-only"
    );
}

#[test]
fn no_checks_is_an_error_not_a_healthy_report() {
    let (doctor, _) = use_case(Ok(config(&["/bin/create"])), Ok(vec![]));
    let error = doctor
        .execute(&ContainerRuntimeTarget::default())
        .unwrap_err();
    assert_eq!(
        error,
        DiagnoseContainerRuntimeError::PreflightUnavailable {
            config: "official".into(),
            detail: "create script /bin/create reported no preflight checks".into(),
        }
    );
}

#[test]
fn an_empty_create_argv_never_reaches_the_preflight() {
    let (doctor, preflight) = use_case(Ok(config(&[])), Ok(vec![check("x", CheckStatus::Passed)]));
    let error = doctor
        .execute(&ContainerRuntimeTarget::default())
        .unwrap_err();
    assert!(
        matches!(
            error,
            DiagnoseContainerRuntimeError::PreflightUnavailable { .. }
        ),
        "{error}"
    );
    assert!(preflight.seen.lock().unwrap().is_empty());
}

#[test]
fn check_status_parses_the_wire_words_only() {
    assert_eq!(CheckStatus::parse("ok"), Some(CheckStatus::Passed));
    assert_eq!(CheckStatus::parse("warn"), Some(CheckStatus::Warned));
    assert_eq!(CheckStatus::parse("fail"), Some(CheckStatus::Failed));
    assert_eq!(CheckStatus::parse("OK"), None);
    assert_eq!(CheckStatus::parse(""), None);
}
