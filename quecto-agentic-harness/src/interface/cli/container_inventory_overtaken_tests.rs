//! #2190 (L4): `quecto container ls|kill|gc` restores as `cli`, which
//! created none of the environments it lists, so a correction another
//! quecto process overtook is reported whoever created the environment.

use super::tests::{run, script};
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::interface::cli::CliContext;

/// A registry with `C1` recorded `running` (no members: listed `empty`) by
/// session `cli:qa`. Its
/// inspect is the race: while the restore checks C1 (after its load, before
/// its correction) another quecto process retains it on file, and the
/// container answers that it exited.
fn racing_registry() -> (tempfile::TempDir, CliContext) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().join("base");
    std::fs::create_dir_all(&base).unwrap();
    let workspace = dir.path().join("state/env-one/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(&base);
    // The other process's retention, as the file it leaves (written once
    // the registry holds C1 running, below): copied over the registry by an
    // inspect that finds C1 still running, as the other process retains
    // only a running record. C1 is the registry's one environment, so its
    // status is the file's one `status` (asserted below).
    let overtaken = dir.path().join("overtaken.json");
    let inspect = script(
        dir.path(),
        "inspect.sh",
        &format!(
            r#"if grep -Eq '"status": ?"running"' '{registry}'; then cp '{overtaken}' '{registry}'; fi
printf '{{"status":"exited","metadata":{{}}}}'"#,
            overtaken = overtaken.display(),
            registry = store.path().display()
        ),
    );
    store
        .record(&EnvironmentRecord {
            environment_ref: "C1".into(),
            environment_id: "env-one".into(),
            environment_uuid: "uuid-C1".into(),
            name: Some("name-C1".into()),
            workspace_path: workspace,
            repository: "https://example.test/repo".into(),
            script_name: "official".into(),
            retained_exec_argv: vec!["exec".into()],
            retained_kill_argv: vec!["kill".into()],
            retained_cleanup_argv: vec![],
            retained_inspect_argv: inspect,
            members: vec![],
            status: EnvironmentStatus::Running,
            metadata: serde_json::json!({}),
            last_error: None,
            origin: EnvironmentOrigin::Created,
            created_by: "cli:qa".into(),
            created_at: Some(0),
        })
        .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(
        document["environments"]["C1"]["status"], "running",
        "{document}"
    );
    let environments = document["environments"].as_object().map(|all| all.len());
    assert_eq!(environments, Some(1), "C1 alone: {document}");
    let record = &mut document["environments"]["C1"];
    record["status"] = serde_json::json!("retained");
    record["metadata"] = serde_json::json!({"retained": "kept by its owner"});
    std::fs::write(&overtaken, document.to_string()).unwrap();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let ctx = CliContext {
        base_dir: Some(base),
        cwd: Some(cwd),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_inventory: Some(crate::composition::environments::build_container_inventory),
        ..Default::default()
    };
    (dir, ctx)
}

#[test]
fn container_ls_reports_a_correction_another_process_overtook_on_anyones_environment() {
    let (_dir, ctx) = racing_registry();
    let output = run(&["container", "ls", "--all"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(
        output.stderr.contains(
            "C1 changed while it was being checked (empty → retained): \
             another quecto process changed it; its state stands"
        ),
        "{output:?}"
    );
    let on_file = FileEnvironmentRegistryStore::for_base_dir(&ctx.base_dir())
        .load()
        .unwrap();
    assert_eq!(
        on_file[0].status,
        EnvironmentStatus::Retained,
        "what the other process wrote stands"
    );
}
