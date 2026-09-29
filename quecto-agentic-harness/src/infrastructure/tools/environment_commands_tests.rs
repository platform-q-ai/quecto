//! Script adapters of the environments capability's ports (#1939).
use crate::application::environments::ports::EnvironmentProcessCommands;
use crate::infrastructure::test_support::executable::write_executable;

#[test]
fn run_kill_sync_reports_missing_argv_and_failures_truthfully() {
    assert!(
        super::run_kill_sync("env-x", &[], super::KILL_SCRIPT_BOUND)
            .unwrap_err()
            .contains("no retained kill argv")
    );
    assert!(
        super::run_kill_sync("env-x", &["false".to_string()], super::KILL_SCRIPT_BOUND)
            .unwrap_err()
            .contains("retained kill exited")
    );
    assert!(
        super::run_kill_sync(
            "env-x",
            &["/definitely/not/a/kill".to_string()],
            super::KILL_SCRIPT_BOUND
        )
        .unwrap_err()
        .contains("failed to invoke"),
    );
    assert!(super::run_kill_sync("env-x", &["true".to_string()], super::KILL_SCRIPT_BOUND).is_ok());
}

/// #1391 review: the inspect subprocess is bounded — a hung script is killed
/// and reported as a timeout instead of stalling the death pipeline. The
/// bound is the shared script runner's (#2024 S4b); this pins the inspect
/// path's own wording, which names the retry.
#[test]
fn inspect_kills_hung_scripts_and_names_the_retry() {
    let started = std::time::Instant::now();
    let err = super::run_inspect_sync_bounded(
        "env-x",
        &["sleep".to_string(), "30".to_string()],
        std::time::Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(err.contains("retained argv kept for retry"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(super::INSPECT_TIMEOUT >= std::time::Duration::from_secs(1));
}

/// The kill adapter answers the port truthfully whether or not a runtime
/// is present: success only when the script reports it.
#[tokio::test]
async fn script_kill_adapter_reports_the_script_outcome_on_a_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kill.log");
    let script = temp.path().join("kill.sh");
    write_executable(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\nexit ${{KILL_EXIT:-0}}\n",
            log.display()
        ),
    );
    let argv = vec!["bash".to_string(), script.to_string_lossy().to_string()];
    super::ScriptEnvironmentCommands::default()
        .run_retained_kill("env-runtime", &argv)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "env-runtime");
    let err = super::ScriptEnvironmentCommands::default()
        .run_retained_kill("env-runtime", &[])
        .await
        .unwrap_err();
    assert!(err.contains("no retained kill argv"), "{err}");
    super::ScriptEnvironmentCommands::default()
        .run_retained_cleanup("env-runtime", &argv)
        .await
        .expect("the cleanup script reported success");
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        2,
        "cleanup ran the same script once more"
    );
    let err = super::ScriptEnvironmentCommands::default()
        .run_retained_cleanup("env-runtime", &[])
        .await
        .unwrap_err();
    assert!(err.contains("no retained cleanup argv"), "{err}");
}

/// A cleanup that hangs is killed at the adapter's bound and reported
/// (#2206): an owner's end waiting on it is never parked.
#[tokio::test]
async fn a_hung_cleanup_is_killed_at_the_bound_and_reported() {
    let argv = vec!["sleep".to_string(), "30".to_string()];
    let started = std::time::Instant::now();
    let err = super::ScriptEnvironmentCommands::default()
        .with_cleanup_bound(std::time::Duration::from_millis(200))
        .run_retained_cleanup("env-hung", &argv)
        .await
        .unwrap_err();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "bounded"
    );
    assert!(!err.is_empty());
}

/// Without a runtime the adapter runs the script inline (the final-member
/// path runs on a blocking worker under `block_on`).
#[test]
fn script_adapters_run_inline_without_a_runtime() {
    let ok = futures::executor::block_on(
        super::ScriptEnvironmentCommands::default()
            .run_retained_kill("env-x", &["true".to_string()]),
    );
    assert!(ok.is_ok());
    let err = futures::executor::block_on(
        super::ScriptEnvironmentCommands::default().run_retained_inspect("env-x", &[]),
    )
    .unwrap_err();
    assert!(err.contains("no retained inspect argv"), "{err}");
}

/// Review round 2 (#2024 S4e): the retained argv is judged like the create's
/// — a standard-bundle script that no longer carries the embedded bytes is
/// refused naming the file, and the altered script never runs, on the
/// inspect, kill and cleanup paths alike.
#[tokio::test]
async fn retained_argv_of_an_altered_standard_script_is_refused_before_it_runs() {
    use crate::infrastructure::processes::containers::standard::integrity::test_support::{
        alter_script, materialise_bundle,
    };
    let project = tempfile::tempdir().unwrap();
    let bundle = materialise_bundle(project.path());
    let marker = project.path().join("ran");
    let commands = super::ScriptEnvironmentCommands::default();
    for (name, op) in [("kill.sh", "kill"), ("inspect.sh", "inspect")] {
        let script = bundle.join("scripts").join(name);
        alter_script(&script, &marker);
        let argv = vec![
            script.to_string_lossy().into_owned(),
            "--state-dir".to_string(),
            project.path().join("state").to_string_lossy().into_owned(),
            "--op".to_string(),
            op.to_string(),
        ];
        let err = match op {
            "kill" => commands
                .run_retained_kill("env-1", &argv)
                .await
                .unwrap_err(),
            _ => commands
                .run_retained_inspect("env-1", &argv)
                .await
                .unwrap_err(),
        };
        assert!(
            err.contains(&format!(
                "{} differs from the standard bundle this quecto embeds",
                script.display()
            )),
            "{op}: {err}"
        );
        assert!(
            err.contains("quecto container init --refresh"),
            "{op}: {err}"
        );
        assert!(!marker.exists(), "the altered {name} ran on the host");
        if op == "kill" {
            let refused = commands.run_retained_cleanup("env-1", &argv).await;
            assert!(refused.is_err(), "an altered cleanup is reported refused");
            assert!(!marker.exists(), "the altered {name} ran as cleanup");
        }
    }
}

/// A kill script that hangs is killed at the bound and reported (#2070):
/// the record ends `cleanup-failed` with the reason, never a parked harness.
#[tokio::test]
async fn a_hung_kill_script_is_killed_at_the_bound_and_reported() {
    use crate::application::environments::ports::EnvironmentProcessCommands;
    let started = std::time::Instant::now();
    let error = super::ScriptEnvironmentCommands::default()
        .with_kill_bound(std::time::Duration::from_millis(200))
        .run_retained_kill("env-x", &["sleep".to_string(), "30".to_string()])
        .await
        .unwrap_err();
    assert!(
        error.contains("timed out") && error.contains("was killed"),
        "{error}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

/// #2206: a cleanup's bound is its own, generous one — a large state
/// directory's `rm -rf` must not be cut at the kill's 20 s.
#[test]
fn a_cleanup_is_bounded_generously_and_apart_from_the_kill() {
    assert!(super::CLEANUP_SCRIPT_BOUND >= std::time::Duration::from_secs(120));
    assert!(super::CLEANUP_SCRIPT_BOUND > super::KILL_SCRIPT_BOUND);
    // A cleanup that outlives the kill's bound still completes.
    let argv = vec!["sleep".to_string(), "0.4".to_string()];
    futures::executor::block_on(
        super::ScriptEnvironmentCommands::default()
            .with_kill_bound(std::time::Duration::from_millis(100))
            .run_retained_cleanup("env-slow", &argv),
    )
    .expect("the kill's bound does not apply to a cleanup");
}

/// The #2206 round-1 probe, through `kill_container` of a stopped ref and
/// the official scripts: a runtime that cannot be asked is no proof the
/// container is gone, so nothing is removed and the record stays; one that
/// says "no such container" is, so the leftovers go and the record is
/// forgotten.
#[test]
fn a_stopped_ref_is_removed_only_on_the_runtimes_own_word() {
    use crate::application::environments::ports::{
        EnvironmentMemberShutdown, MemberShutdownReport, PortFuture,
    };
    use crate::application::environments::use_cases::KillEnvironment;
    use crate::domain::environment_registry::{
        EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
        EnvironmentTarget,
    };
    struct NoMembers;
    impl EnvironmentMemberShutdown for NoMembers {
        fn shutdown_members<'a>(&'a self, _: &'a [String]) -> PortFuture<'a, MemberShutdownReport> {
            Box::pin(async { MemberShutdownReport::default() })
        }
    }
    let scripts =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/standard-container/scripts");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    for (answer, removed) in [
        (
            "echo 'Error: cannot connect to Podman socket' >&2\nexit 125",
            false,
        ),
        (
            "echo 'Error: no such container quecto-env-x' >&2\nexit 125",
            true,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let env_dir = state.join("env-x");
        std::fs::create_dir_all(env_dir.join("workspace/repo")).unwrap();
        std::fs::write(env_dir.join("container"), "quecto-env-x\n").unwrap();
        std::fs::write(env_dir.join("workspace/repo/work.txt"), "work\n").unwrap();
        let cli = dir.path().join("podman");
        write_executable(&cli, format!("#!/bin/sh\n{answer}\n"));
        // The fake runtime reaches only these scripts: no process-wide
        // environment is touched.
        let argv = |script: &str, rest: &[&str]| -> Vec<String> {
            let mut argv = vec![
                "env".to_string(),
                format!("QUECTO_CONTAINER_CLI={}", cli.display()),
                "bash".to_string(),
                scripts.join(script).display().to_string(),
                "--state-dir".to_string(),
                state.display().to_string(),
            ];
            argv.extend(rest.iter().map(|s| s.to_string()));
            argv
        };
        let registry = EnvironmentRegistry::new();
        registry.commit(EnvironmentRecord {
            environment_ref: "C7".into(),
            environment_id: "env-x".into(),
            environment_uuid: "u7".into(),
            name: None,
            workspace_path: env_dir.join("workspace"),
            repository: String::new(),
            script_name: "standard".into(),
            retained_exec_argv: vec!["exec.sh".into()],
            retained_kill_argv: argv("kill.sh", &["--op", "kill"]),
            retained_cleanup_argv: argv("kill.sh", &["--op", "cleanup"]),
            retained_inspect_argv: argv("inspect.sh", &[]),
            members: vec![],
            status: EnvironmentStatus::Stopped,
            metadata: serde_json::json!({}),
            last_error: None,
            origin: EnvironmentOrigin::Restored,
            created_by: "cli".into(),
            created_at: None,
        });
        let kill = KillEnvironment::new(
            registry.clone(),
            std::sync::Arc::new(NoMembers),
            std::sync::Arc::new(super::ScriptEnvironmentCommands::default()),
            std::sync::Arc::new(super::HostedStoreObservation::new(
                crate::composition::swarm::swarm_board(),
            )),
        );

        let outcome = rt.block_on(kill.kill_container(&EnvironmentTarget::Ref("C7".into())));

        if removed {
            assert!(outcome.expect("removed").removed_stopped);
            assert!(!env_dir.exists(), "leftovers removed");
            assert!(registry.get("C7").is_none(), "record forgotten");
        } else {
            let error = outcome.unwrap_err().to_string();
            assert!(error.contains("could not be checked"), "{error}");
            assert!(
                env_dir.join("workspace/repo/work.txt").exists(),
                "work kept"
            );
            assert_eq!(
                registry.get("C7").unwrap().status,
                EnvironmentStatus::Stopped,
                "record untouched"
            );
        }
    }
}
