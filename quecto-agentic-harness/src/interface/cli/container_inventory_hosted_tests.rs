//! `quecto container ls|gc|kill` over an environment whose master exited
//! before its coordinator did (round 4 M1, #2033): the record says
//! `running`, the container has exited, and the checkout still hosts a
//! coordination store with an unfinished run. The restore relabels it
//! `retained` (never `stopped`), `gc` keeps it whether it previews or
//! collects, and only `container kill` ends it.
use super::tests::{composed_with_exited_containers, run};
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::EnvironmentStatus;
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use std::sync::Arc;

/// A running run created by "coordinator" in the store at `checkout`.
fn create_running_swarm(checkout: &std::path::Path) -> String {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    context
        .call(
            "create",
            serde_json::json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], 3, deadline]),
        )
        .unwrap();
    crate::infrastructure::tools::swarm_bridge::HostedStore::at(checkout.to_path_buf())
        .hosted_run()
        .unwrap()
        .unwrap()
        .id
}

#[test]
fn an_exited_box_hosting_an_unfinished_run_is_retained_by_ls_kept_by_gc_and_ended_by_kill() {
    // `C3` is recorded running, its container exited, its checkout
    // `<state>/env-three/workspace` hosts a running run.
    let (dir, ctx, cleanup_log) = composed_with_exited_containers();
    let checkout = dir.path().join("state/env-three/workspace");
    let run_id = create_running_swarm(&checkout);
    let store = FileEnvironmentRegistryStore::for_base_dir(&ctx.base_dir());

    let output = run(&["container", "ls"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(
        output.stdout.contains("C3   name-C3  official  retained"),
        "{}",
        output.stdout
    );
    let note = format!(
        "note: C3 retained at restore: run {run_id} unfinished (running); container exited; environment retained for inspection, kill_container to remove"
    );
    assert!(output.stderr.contains(&note), "{}", output.stderr);
    let on_file = |reference: &str| {
        store
            .load()
            .unwrap()
            .into_iter()
            .find(|r| r.environment_ref == reference)
            .unwrap()
    };
    let c3 = on_file("C3");
    assert_eq!(c3.status, EnvironmentStatus::Retained);
    assert_eq!(c3.last_error, None);
    assert!(
        c3.metadata["retained"]
            .as_str()
            .unwrap()
            .starts_with(&format!("run {run_id} unfinished")),
        "{c3:?}"
    );

    // Neither a preview nor a real collection touches it.
    for args in [
        &["container", "gc", "--dry-run"][..],
        &["container", "gc"][..],
    ] {
        let output = run(args, &ctx);
        assert_eq!(output.exit_code, 0, "{output:?}");
        assert!(
            output.stdout.contains("env-three  recorded C3 as retained")
                && output.stdout.contains("quecto container kill C3"),
            "{}",
            output.stdout
        );
        assert!(
            !output.stdout.contains("via retained cleanup of C3"),
            "{}",
            output.stdout
        );
    }
    assert!(!cleanup_log.exists(), "no cleanup ran");
    assert!(checkout.join(".quecto/swarm.sqlite").is_file());
    assert_eq!(on_file("C3").status, EnvironmentStatus::Retained);

    // An explicit kill ends it: the record is stopped and its leftovers
    // are the collector's.
    let output = run(&["container", "kill", "C3"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(on_file("C3").status, EnvironmentStatus::Stopped);
}
