use super::*;
use crate::application::environments::dto::RestoreMode;
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::EnvironmentOrigin;
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::interface::cli::CliOutput;

pub(super) fn run(args: &[&str], ctx: &CliContext) -> CliOutput {
    let mut argv = vec!["quecto".to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    crate::interface::cli::run_with_output(argv, ctx)
}

pub(super) fn script(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
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
    let request = parse_gc(&["--dry-run".into(), "--name".into(), "box".into()]).unwrap();
    assert!(request.dry_run);
    assert_eq!(request.config.as_deref(), Some("box"));
    assert!(
        parse_gc(&["--state-dir".into(), "/s".into()])
            .unwrap_err()
            .contains("unknown argument --state-dir")
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
    let handles = (ctx.container_inventory.unwrap())(
        &ctx.base_dir(),
        &ctx.config_selection().unwrap(),
        RestoreMode::Correct,
    );
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

/// Review F3 (#2033): an unreadable registry is not an empty one. `gc`
/// must refuse — every exited state dir would otherwise read as "no
/// registry record" and be collected, including one a live session
/// records as retained — and `ls` must fail rather than print an empty
/// table; both name the read error on stderr.
#[test]
fn an_unreadable_registry_refuses_gc_and_fails_ls_naming_the_read_error() {
    let (_dir, ctx, _) = composed();
    std::fs::write(ctx.base_dir().join("environments.json"), "{not json").unwrap();
    let output = run(&["container", "gc", "--dry-run"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output
            .stderr
            .contains("durable environment registry could not be read"),
        "{output:?}"
    );
    assert!(
        output
            .stderr
            .contains("is not a valid environment registry"),
        "{output:?}"
    );
    assert!(!output.stdout.contains("would remove"), "{output:?}");
    assert!(!output.stdout.contains("orphaned"), "{output:?}");
    let output = run(&["container", "ls"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output
            .stderr
            .contains("durable environment registry could not be read"),
        "{output:?}"
    );
    assert!(
        !output.stdout.contains("no live environments"),
        "{output:?}"
    );
    let output = run(&["container", "kill", "C1"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output
            .stderr
            .contains("durable environment registry could not be read"),
        "{output:?}"
    );
}

/// A base dir like [`composed`]'s whose inspect reports every container
/// dead, holding a `retained` `C1` and a `running` `C3` (both with state
/// dirs), and a global config whose `official` container config lists
/// nothing and removes through a recording cleanup.
pub(super) fn composed_with_exited_containers()
-> (tempfile::TempDir, CliContext, std::path::PathBuf) {
    let (dir, mut ctx, _) = composed();
    let base = ctx.base_dir();
    let state = dir.path().join("state");
    std::fs::create_dir_all(state.join("env-three/workspace")).unwrap();
    std::fs::write(state.join("env-three/container"), "quecto-env-three\n").unwrap();
    let dead = script(
        dir.path(),
        "inspect.sh",
        r#"if [ "${1:-}" = --list ]; then exit 0; fi; printf '{"status":"dead","metadata":{}}'"#,
    );
    let cleanup_log = dir.path().join("cleanup.log");
    let cleanup = script(
        dir.path(),
        "cleanup.sh",
        &format!(
            "echo \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'; rm -rf '{}'/\"$QUECTO_CONTAINER_ENVIRONMENT_ID\"",
            cleanup_log.display(),
            state.display()
        ),
    );
    let store = FileEnvironmentRegistryStore::for_base_dir(&base);
    let mut records = store.load().unwrap();
    let mut c3 = records[0].clone();
    c3.environment_ref = "C3".into();
    c3.environment_id = "env-three".into();
    c3.name = Some("name-C3".into());
    c3.workspace_path = state.join("env-three/workspace");
    for record in records.iter_mut() {
        record.retained_inspect_argv = dead.clone();
        if record.environment_ref == "C1" {
            record.status = EnvironmentStatus::Retained;
        }
    }
    c3.retained_inspect_argv = dead.clone();
    c3.retained_cleanup_argv = cleanup.clone();
    for record in records.iter().chain(std::iter::once(&c3)) {
        store.record(record).unwrap();
    }
    std::fs::write(
        base.join("config.json"),
        serde_json::json!({"container_configs": {"official": {
            "default": true,
            "create": ["true", "--state-dir", state.to_string_lossy()],
            "exec": ["true"],
            "inspect": dead,
            "cleanup": cleanup,
        }}})
        .to_string(),
    )
    .unwrap();
    ctx.config_path = None;
    (dir, ctx, cleanup_log)
}

/// Round 3 H1 (#2033): a retained environment's container has exited by
/// design; `ls` still lists it (as `retained`, without `--all`) and no
/// command relabels it.
#[test]
fn ls_lists_a_retained_environment_whose_container_exited() {
    let (_dir, ctx, _) = composed_with_exited_containers();
    let output = run(&["container", "ls"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(
        output.stdout.contains("C1   name-C1  official  retained"),
        "{}",
        output.stdout
    );
    assert!(
        output
            .stderr
            .contains("note: C1 could not be verified against the runtime: retained: container exited; only container kill ends it"),
        "{}",
        output.stderr
    );
    let on_file = FileEnvironmentRegistryStore::for_base_dir(&ctx.base_dir())
        .load()
        .unwrap();
    assert_eq!(on_file[0].status, EnvironmentStatus::Retained);
}

/// Round 3 H1 (#2033): `gc --dry-run` has no effect at all — the restore
/// it runs over previews its corrections (`C3` running → stopped, its
/// container gone) without writing them, so `environments.json` is byte
/// for byte what it was; a real `gc` writes the correction and collects
/// through `C3`'s own cleanup, and keeps the retained `C1` either way.
#[test]
fn gc_dry_run_leaves_the_registry_document_untouched_and_a_real_gc_corrects_it() {
    let (_dir, ctx, cleanup_log) = composed_with_exited_containers();
    let document = ctx.base_dir().join("environments.json");
    let before = std::fs::read(&document).unwrap();
    let output = run(&["container", "gc", "--dry-run"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(
        output.stdout.contains("env-three  ")
            && output.stdout.contains("via retained cleanup of C3"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("env-one  recorded C1 as retained"),
        "{}",
        output.stdout
    );
    assert!(
        output.stderr.contains("C3 would be recorded stopped"),
        "{}",
        output.stderr
    );
    assert_eq!(
        std::fs::read(&document).unwrap(),
        before,
        "a dry run writes nothing"
    );
    assert!(!cleanup_log.exists(), "a dry run removes nothing");
    let output = run(&["container", "gc"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(
        std::fs::read_to_string(&cleanup_log).unwrap().trim(),
        "env-three"
    );
    let on_file = FileEnvironmentRegistryStore::for_base_dir(&ctx.base_dir())
        .load()
        .unwrap();
    let statuses: Vec<(&str, &EnvironmentStatus)> = on_file
        .iter()
        .map(|r| (r.environment_ref.as_str(), &r.status))
        .collect();
    assert_eq!(
        statuses,
        [("C1", &EnvironmentStatus::Retained)],
        "C3 was collected and forgotten (C2, stopped with nothing left, likewise); the retained C1 stands"
    );
    assert!(
        output.stdout.contains("env-one  recorded C1 as retained"),
        "{}",
        output.stdout
    );
}

#[test]
fn gc_parses_the_abandoned_policy_and_refuses_a_bad_duration() {
    use crate::application::environments::dto::AbandonedRuns;
    assert_eq!(parse_gc(&[]).unwrap().abandoned, AbandonedRuns::Keep);
    assert_eq!(
        parse_gc(&["--abandoned".into()]).unwrap().abandoned,
        AbandonedRuns::Collect
    );
    for (spelled, secs) in [
        ("45s", 45),
        ("30m", 1_800),
        ("12h", 43_200),
        ("3d", 259_200),
    ] {
        assert_eq!(
            parse_gc(&["--abandoned-after".into(), spelled.into()])
                .unwrap()
                .abandoned,
            AbandonedRuns::OlderThan { secs },
            "{spelled}"
        );
    }
    for bad in [
        "",
        "3",
        "3w",
        "h",
        "1.5h",
        "-3d",
        "3 d",
        "99999999999999999999d",
    ] {
        let error = parse_gc(&["--abandoned-after".into(), bad.into()]).unwrap_err();
        assert!(error.contains("--abandoned-after"), "{bad:?}: {error}");
    }
    let twice = parse_gc(&[
        "--abandoned".into(),
        "--abandoned-after".into(),
        "1d".into(),
    ])
    .unwrap_err();
    assert!(twice.contains("may be given once"), "{twice}");
}
