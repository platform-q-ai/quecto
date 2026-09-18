use super::*;
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::EnvironmentOrigin;
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::interface::cli::CliOutput;

fn run(args: &[&str], ctx: &CliContext) -> CliOutput {
    let mut argv = vec!["quecto".to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    crate::interface::cli::run_with_output(argv, ctx)
}

fn script(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

/// A base dir holding a durable registry with a running `C1` (whose
/// inspect script says running, whose kill records itself) and a stopped
/// `C2`, plus a fake `podman` on a private PATH listing nothing.
fn composed() -> (tempfile::TempDir, CliContext, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().join("base");
    std::fs::create_dir_all(&base).unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(state.join("env-one/workspace")).unwrap();
    std::fs::write(state.join("env-one/container"), "quecto-env-one\n").unwrap();
    let inspect = script(
        dir.path(),
        "inspect.sh",
        r#"printf '{"status":"running","metadata":{}}'"#,
    );
    let kill_log = dir.path().join("kill.log");
    let kill = script(
        dir.path(),
        "kill.sh",
        &format!(
            "echo \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'",
            kill_log.display()
        ),
    );
    let store = FileEnvironmentRegistryStore::for_base_dir(&base);
    let record = |reference: &str, id: &str, status: EnvironmentStatus| EnvironmentRecord {
        environment_ref: reference.to_string(),
        environment_id: id.to_string(),
        environment_uuid: format!("uuid-{reference}"),
        name: Some(format!("name-{reference}")),
        workspace_path: state.join(id).join("workspace"),
        repository: "https://user:secret@example.test/repo".into(),
        script_name: "official".into(),
        retained_exec_argv: vec!["exec".into()],
        retained_kill_argv: kill.clone(),
        retained_cleanup_argv: vec![],
        retained_inspect_argv: inspect.clone(),
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: "cli:default".into(),
        created_at: Some(0),
    };
    store
        .record(&record("C1", "env-one", EnvironmentStatus::Running))
        .unwrap();
    store
        .record(&record("C2", "env-two", EnvironmentStatus::Stopped))
        .unwrap();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let ctx = CliContext {
        base_dir: Some(base),
        cwd: Some(cwd),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_inventory: Some(crate::composition::environments::build_container_inventory),
        ..Default::default()
    };
    (dir, ctx, kill_log)
}

#[test]
fn ls_lists_live_environments_in_a_table_and_all_includes_stopped_ones() {
    let (_dir, ctx, _) = composed();
    let output = run(&["container", "ls"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    let lines: Vec<&str> = output.stdout.lines().collect();
    assert!(lines[0].starts_with("REF  NAME"), "{}", output.stdout);
    assert!(
        lines[1].starts_with("C1   name-C1  official  empty"),
        "{}",
        output.stdout
    );
    assert!(
        lines[1].contains("https://***@example.test/repo"),
        "{}",
        output.stdout
    );
    assert!(!output.stdout.contains("secret"), "{}", output.stdout);
    assert!(lines[1].contains("cli:default"), "{}", output.stdout);
    assert!(!output.stdout.contains("C2 "), "{}", output.stdout);
    assert!(
        output
            .stdout
            .contains("(1 stopped environment hidden; --all shows it)"),
        "{}",
        output.stdout
    );
    let output = run(&["container", "ls", "--all"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(
        output.stdout.contains("C2   name-C2  official  stopped"),
        "{}",
        output.stdout
    );
    let output = run(&["container", "ls", "--bogus"], &ctx);
    assert_eq!(output.exit_code, 1);
    assert!(
        output.stderr.contains("unknown argument --bogus"),
        "{output:?}"
    );
}

#[test]
fn ls_on_an_empty_base_dir_says_so() {
    let dir = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(dir.path().to_path_buf()),
        cwd: Some(dir.path().to_path_buf()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_inventory: Some(crate::composition::environments::build_container_inventory),
        ..Default::default()
    };
    let output = run(&["container", "ls"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(output.stdout.contains("no live environments"), "{output:?}");
}

#[test]
fn kill_by_ref_or_name_runs_the_retained_kill_and_records_stopped() {
    let (_dir, ctx, kill_log) = composed();
    let output = run(&["container", "kill", "name-C1"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(output.stdout, "killed C1 (name-C1)\n");
    assert_eq!(std::fs::read_to_string(&kill_log).unwrap(), "env-one\n");
    let output = run(&["container", "kill", "C1"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output.stderr.contains("environment 'C1' is stopped"),
        "{output:?}"
    );
    let output = run(&["container", "kill", "C9"], &ctx);
    assert!(
        output.stderr.contains("environment 'C9' is unknown"),
        "{output:?}"
    );
    let output = run(&["container", "kill"], &ctx);
    assert_eq!(output.exit_code, 1);
    assert!(
        output.stderr.contains("kill takes exactly one <ref|name>"),
        "{output:?}"
    );
}

#[test]
fn commands_refuse_without_a_composed_inventory() {
    let ctx = CliContext::default();
    for args in [
        vec!["container", "ls"],
        vec!["container", "kill", "C1"],
        vec!["container", "gc", "--dry-run"],
    ] {
        let output = run(&args, &ctx);
        assert_eq!(output.exit_code, 1, "{output:?}");
        assert!(
            output.stderr.contains("container inventory not composed"),
            "{output:?}"
        );
    }
}

#[test]
fn gc_parses_dry_run_name_and_state_dirs_and_refuses_the_rest() {
    let request = parse_gc(&[
        "--dry-run".into(),
        "--name".into(),
        "box".into(),
        "--state-dir".into(),
        "/s".into(),
    ])
    .unwrap();
    assert!(request.dry_run);
    assert_eq!(request.config.as_deref(), Some("box"));
    assert_eq!(request.state_roots, vec![std::path::PathBuf::from("/s")]);
    assert!(
        parse_gc(&["--state-dir".into()])
            .unwrap_err()
            .contains("--state-dir requires")
    );
    assert!(
        parse_gc(&["--name".into()])
            .unwrap_err()
            .contains("--name requires")
    );
    assert!(
        parse_gc(&["--name".into(), "a".into(), "--name".into(), "b".into()])
            .unwrap_err()
            .contains("--name may be given once")
    );
    assert!(
        parse_gc(&["--force".into()])
            .unwrap_err()
            .contains("unknown argument --force")
    );
}

#[test]
fn gc_without_a_container_config_is_refused_in_the_collectors_words() {
    let (_dir, ctx, _) = composed();
    let output = run(&["container", "gc", "--dry-run"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(output.stderr.contains("container config"), "{output:?}");
}

#[test]
fn ages_are_compact() {
    assert_eq!(age(5), "5s");
    assert_eq!(age(600), "10m");
    assert_eq!(age(7200), "2h");
    assert_eq!(age(200_000), "2d");
}

#[test]
fn gc_report_presents_candidates_kept_and_roots() {
    let report = GcReport {
        dry_run: true,
        config: "official".into(),
        state_roots: vec!["/s".into()],
        removable: vec![
            GcCandidate {
                environment_id: "env-a".into(),
                state_dir: Some("/s/env-a".into()),
                container: Some("quecto-env-a".into()),
                removal: GcRemoval::ConfiguredCleanup {
                    config: "official".into(),
                },
                reason: "container quecto-env-a exited; no registry record".into(),
            },
            GcCandidate {
                environment_id: "env-b".into(),
                state_dir: None,
                container: Some("quecto-env-b".into()),
                removal: GcRemoval::RetainedCleanup {
                    environment_ref: "C3".into(),
                },
                reason: "recorded C3 as stopped".into(),
            },
        ],
        removed: vec![],
        kept: vec![crate::application::environments::dto::GcKept {
            environment_id: "env-c".into(),
            reason: "container quecto-env-c is running".into(),
        }],
        errors: vec![],
    };
    let mut out = String::new();
    present_gc(&report, &mut out);
    assert!(
        out.contains("would remove 2 orphaned environments:"),
        "{out}"
    );
    assert!(out.contains("container config \"official\"\n"), "{out}");
    assert!(out.contains("  env-a  /s/env-a + container quecto-env-a  [container quecto-env-a exited; no registry record; via cleanup of config 'official']"), "{out}");
    assert!(out.contains("  env-b  container quecto-env-b (no state dir)  [recorded C3 as stopped; via retained cleanup of C3]"), "{out}");
    assert!(
        out.contains("kept 1 environment:\n  env-c  container quecto-env-c is running"),
        "{out}"
    );
}

#[test]
fn the_inventory_handles_debug_shows_the_restore_only() {
    let (_dir, ctx, _) = composed();
    let handles =
        (ctx.container_inventory.unwrap())(&ctx.base_dir(), &ctx.config_selection().unwrap());
    let shown = format!("{handles:?}");
    assert!(shown.starts_with("ContainerInventoryHandles"), "{shown}");
    assert!(shown.contains("restore"), "{shown}");
    assert_eq!(handles.restore.restored, ["C1"]);
}

#[test]
fn a_real_gc_run_presents_removed_and_kept_entries() {
    let report = GcReport {
        dry_run: false,
        config: "official".into(),
        state_roots: vec![],
        removable: vec![GcCandidate {
            environment_id: "env-a".into(),
            state_dir: None,
            container: None,
            removal: GcRemoval::ConfiguredCleanup {
                config: "official".into(),
            },
            reason: "no container recorded; no registry record".into(),
        }],
        removed: vec![],
        kept: vec![],
        errors: vec!["env-a: cleanup exited 1".into()],
    };
    let mut out = String::new();
    present_gc(&report, &mut out);
    assert!(out.contains("scanned no state roots"), "{out}");
    assert!(
        out.contains("removed 0 of 1 orphaned environment:"),
        "{out}"
    );
    assert!(!out.contains("nothing on disk"), "{out}");
    let mut removed = report.clone();
    removed.removed = removed.removable.clone();
    removed.errors.clear();
    let mut out = String::new();
    present_gc(&removed, &mut out);
    assert!(
        out.contains("removed 1 of 1 orphaned environment:\n  env-a  nothing on disk"),
        "{out}"
    );
}
