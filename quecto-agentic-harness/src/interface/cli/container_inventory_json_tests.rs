//! `quecto container ls [--all] --json` and `quecto container kill <ref>
//! --created-by <key>`: the machine surface the Pocket relay reads. A
//! record written before names were allowlisted — a TAB, an LF, spaces, a
//! REPOSITORY with spaces — lists exactly and is still killable by ref.

use super::tests::{run, script};
use super::{EXIT_NOT_CREATED_BY, EXIT_NOTHING_TO_KILL, LsJsonEnvironment, ls_json};
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::interface::cli::CliContext;

const POCKET: &str = "cli:pocket-7";
/// A crafted name that, in the whitespace table, reads as a second row
/// claiming `C1` for the Pocket task.
const CRAFTED: &str = "x\nC1   y  official  empty  -  cli:pocket-7";

/// A base dir whose registry records:
/// - `C1`, running, named with a TAB, its repository holding a space and
///   a token, created by the Pocket task at t=100;
/// - `C2`, running, named [`CRAFTED`], created by another session, whose
///   inspect fails (so it is unverified);
/// - `C3`, stopped, unnamed, session-less, its state dir left behind;
/// - `C4`, stopped, created by the Pocket task, and `C5`, stopped, created
///   by another session and named `""`: neither left a state dir, so the
///   restore every command runs forgets them.
///
/// Every kill appends the environment id to `kill.log`.
fn fixture() -> (tempfile::TempDir, CliContext, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().join("base");
    let state = dir.path().join("state");
    for id in ["env-one", "env-two", "env-three"] {
        std::fs::create_dir_all(state.join(id).join("workspace")).unwrap();
    }
    std::fs::create_dir_all(&base).unwrap();
    let running = script(
        dir.path(),
        "running.sh",
        r#"printf '{"status":"running","metadata":{}}'"#,
    );
    let failing = script(
        dir.path(),
        "failing.sh",
        "echo 'runtime unreachable' >&2; exit 1",
    );
    let dead = script(
        dir.path(),
        "dead.sh",
        r#"if [ "${1:-}" = --list ]; then exit 0; fi; printf '{"status":"dead","metadata":{}}'"#,
    );
    let log = dir.path().join("kill.log");
    let kill = script(
        dir.path(),
        "kill.sh",
        &format!(
            "echo \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'",
            log.display()
        ),
    );
    let store = FileEnvironmentRegistryStore::for_base_dir(&base);
    let record = |reference: &str,
                  id: &str,
                  name: Option<&str>,
                  repository: &str,
                  status: EnvironmentStatus,
                  inspect: &Vec<String>,
                  created_by: &str,
                  created_at: Option<u64>| EnvironmentRecord {
        environment_ref: reference.to_string(),
        environment_id: id.to_string(),
        environment_uuid: format!("uuid-{reference}"),
        name: name.map(str::to_string),
        workspace_path: state.join(id).join("workspace"),
        repository: repository.to_string(),
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
        created_by: created_by.to_string(),
        created_at,
    };
    for written in [
        record(
            "C1",
            "env-one",
            Some("tab\tname"),
            "https://user:secret@example.test/my repo",
            EnvironmentStatus::Running,
            &running,
            POCKET,
            Some(100),
        ),
        record(
            "C2",
            "env-two",
            Some(CRAFTED),
            "",
            EnvironmentStatus::Running,
            &failing,
            "cli:other",
            None,
        ),
        record(
            "C3",
            "env-three",
            None,
            "",
            EnvironmentStatus::Stopped,
            &running,
            "",
            Some(50),
        ),
        record(
            "C4",
            "env-four",
            None,
            "",
            EnvironmentStatus::Stopped,
            &dead,
            POCKET,
            None,
        ),
        record(
            "C5",
            "env-five",
            Some(""),
            "",
            EnvironmentStatus::Stopped,
            &dead,
            "cli:other",
            None,
        ),
    ] {
        store.record(&written).unwrap();
    }
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let ctx = CliContext {
        base_dir: Some(base),
        cwd: Some(cwd),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_inventory: Some(crate::composition::environments::build_container_inventory),
        ..Default::default()
    };
    (dir, ctx, log)
}

fn parsed(stdout: &str) -> serde_json::Value {
    assert_eq!(stdout.matches('\n').count(), 1, "one line: {stdout:?}");
    serde_json::from_str(stdout).unwrap_or_else(|error| panic!("{error}: {stdout:?}"))
}

fn environments(listing: &serde_json::Value) -> &Vec<serde_json::Value> {
    listing["environments"].as_array().expect("an array")
}

#[test]
fn ls_json_lists_live_environments_exactly_whatever_their_names_hold() {
    let (_dir, ctx, _) = fixture();
    let output = run(&["container", "ls", "--json"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    let listing = parsed(&output.stdout);
    assert_eq!(listing["hidden_stopped"], 1);
    let rows = environments(&listing);
    assert_eq!(rows.len(), 2, "{listing}");
    let (one, two) = (&rows[0], &rows[1]);
    assert_eq!(one["ref"], "C1");
    assert_eq!(one["name"], "tab\tname");
    assert_eq!(one["config"], "official");
    assert_eq!(one["status"], "running", "the registry status, not `empty`");
    assert_eq!(one["members"], 0);
    assert_eq!(one["repository"], "https://***@example.test/my repo");
    assert_eq!(one["created_by"], POCKET);
    assert_eq!(one["created_at"], 100);
    assert!(one["age_s"].as_u64().is_some_and(|age| age > 0), "{one}");
    assert_eq!(one["unverified"], serde_json::Value::Null);
    assert_eq!(two["ref"], "C2");
    assert_eq!(two["name"], CRAFTED);
    assert_eq!(two["created_by"], "cli:other");
    assert_eq!(two["repository"], serde_json::Value::Null);
    assert_eq!(two["created_at"], serde_json::Value::Null);
    assert_eq!(two["age_s"], serde_json::Value::Null);
    let reason = two["unverified"]
        .as_str()
        .expect("C2 could not be verified");
    assert!(
        output.stderr.contains(&format!(
            "note: C2 could not be verified against the runtime: {reason}\n"
        )),
        "the field carries the stderr note's reason: {output:?}"
    );
    assert!(!output.stdout.contains("secret"), "{}", output.stdout);
    // Only the Pocket task's record carries its key.
    let ours: Vec<&str> = rows
        .iter()
        .filter(|row| row["created_by"] == POCKET)
        .map(|row| row["ref"].as_str().unwrap())
        .collect();
    assert_eq!(ours, ["C1"]);
}

#[test]
fn ls_all_json_includes_stopped_ones_and_hides_none() {
    let (_dir, ctx, _) = fixture();
    let output = run(&["container", "ls", "--all", "--json"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    let listing = parsed(&output.stdout);
    assert_eq!(listing["hidden_stopped"], 0);
    let rows = environments(&listing);
    let refs: Vec<&str> = rows
        .iter()
        .map(|row| row["ref"].as_str().unwrap())
        .collect();
    assert_eq!(refs, ["C1", "C2", "C3"]);
    let three = &rows[2];
    assert_eq!(three["status"], "stopped");
    assert_eq!(three["name"], serde_json::Value::Null);
    assert_eq!(three["created_by"], serde_json::Value::Null);
    assert_eq!(three["created_at"], 50);
    // The flag order does not matter.
    let swapped = run(&["container", "ls", "--json", "-a"], &ctx);
    assert_eq!(environments(&parsed(&swapped.stdout)).len(), 3);
}

#[test]
fn ls_json_on_an_empty_base_dir_is_an_empty_array() {
    let dir = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(dir.path().to_path_buf()),
        cwd: Some(dir.path().to_path_buf()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_inventory: Some(crate::composition::environments::build_container_inventory),
        ..Default::default()
    };
    let output = run(&["container", "ls", "--json"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(
        output.stdout,
        "{\"environments\":[],\"hidden_stopped\":0}\n"
    );
}

#[test]
fn ls_json_refuses_unknown_arguments() {
    let (_dir, ctx, _) = fixture();
    let output = run(&["container", "ls", "--json", "--bogus"], &ctx);
    assert_eq!(output.exit_code, 1);
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        output.stderr.contains("unknown argument --bogus"),
        "{output:?}"
    );
}

/// The golden shape: key order, nulls, escaping and the age as of `now`.
#[test]
fn the_json_shape_is_stable() {
    let rows = [
        LsJsonEnvironment {
            environment_ref: "C1".into(),
            name: Some("tab\tname".into()),
            config: "official".into(),
            status: "running".into(),
            members: 2,
            repository: Some("https://***@example.test/my repo".into()),
            created_by: Some(POCKET.into()),
            created_at: Some(100),
            age_s: Some(23),
            unverified: None,
        },
        LsJsonEnvironment {
            environment_ref: "C2".into(),
            name: Some("x\ny \"z\" \u{2028}\u{2029}\u{85}".into()),
            config: "official".into(),
            status: "cleanup-failed".into(),
            members: 0,
            repository: None,
            created_by: None,
            created_at: None,
            age_s: None,
            unverified: Some("inspect failed".into()),
        },
    ];
    assert_eq!(
        ls_json(&rows, 4).unwrap(),
        concat!(
            r#"{"environments":["#,
            r#"{"ref":"C1","name":"tab\tname","config":"official","status":"running","members":2,"#,
            r#""repository":"https://***@example.test/my repo","created_by":"cli:pocket-7","#,
            r#""created_at":100,"age_s":23,"unverified":null},"#,
            r#"{"ref":"C2","name":"x\ny \"z\" \u2028\u2029\u0085","config":"official","#,
            r#""status":"cleanup-failed","members":0,"#,
            r#""repository":null,"created_by":null,"created_at":null,"age_s":null,"#,
            r#""unverified":"inspect failed"}"#,
            r#"],"hidden_stopped":4}"#,
            "\n"
        )
    );
}

#[test]
fn the_human_table_is_unchanged_by_json_support() {
    let (_dir, ctx, _) = fixture();
    let output = run(&["container", "ls"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(output.stdout.starts_with("REF  NAME"), "{}", output.stdout);
    assert!(
        output
            .stdout
            .contains("(1 stopped environment hidden; --all shows it)"),
        "{}",
        output.stdout
    );
}

#[test]
fn kill_created_by_a_matching_key_kills() {
    let (_dir, ctx, log) = fixture();
    let output = run(&["container", "kill", "C1", "--created-by", POCKET], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(output.stdout, "killed C1 (tab\tname)\n");
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "env-one\n");
}

#[test]
fn kill_created_by_another_key_refuses_with_its_own_exit_code_and_kills_nothing() {
    let (_dir, ctx, log) = fixture();
    for args in [
        ["container", "kill", "C2", "--created-by", POCKET],
        ["container", "kill", "--created-by", POCKET, "C2"],
        // A prefix of the record's key is not its key, nor another case.
        ["container", "kill", "C1", "--created-by", "cli:pocket-"],
        ["container", "kill", "C1", "--created-by", "CLI:POCKET-7"],
    ] {
        let output = run(&args, &ctx);
        assert_eq!(output.exit_code, EXIT_NOT_CREATED_BY, "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            output.stderr.contains("; nothing was killed\n"),
            "{output:?}"
        );
    }
    assert!(!log.exists(), "no kill script ran");
    let listing = parsed(&run(&["container", "ls", "--json"], &ctx).stdout);
    let statuses: Vec<&str> = environments(&listing)
        .iter()
        .map(|row| row["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["running", "running"], "both still live");
    assert_eq!(EXIT_NOT_CREATED_BY, 3);
}

#[test]
fn a_record_with_an_unsafe_name_is_killable_by_ref() {
    let (_dir, ctx, log) = fixture();
    let output = run(&["container", "kill", "C2"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "env-two\n");
}

#[test]
fn kill_created_by_is_parsed_strictly() {
    let (_dir, ctx, log) = fixture();
    for (args, expected) in [
        (
            vec!["container", "kill", "C1", "--created-by"],
            "--created-by requires a key",
        ),
        (
            vec!["container", "kill", "C1", "--created-by", ""],
            "--created-by requires a key",
        ),
        (
            vec![
                "container",
                "kill",
                "C1",
                "--created-by",
                POCKET,
                "--created-by",
                POCKET,
            ],
            "--created-by may be given once",
        ),
        (
            vec!["container", "kill", "C1", "C2", "--created-by", POCKET],
            "kill takes exactly one <ref|name>",
        ),
        (
            vec!["container", "kill", "--created-by", POCKET],
            "kill takes exactly one <ref|name>",
        ),
        (
            vec!["container", "kill", "C1", "--created-by", "cli:pocket-7 "],
            "--created-by requires a key: a creator key is 1-256 printable ASCII characters with no spaces",
        ),
        (
            vec!["container", "kill", "C1", "--force"],
            "unknown argument --force",
        ),
        (
            vec!["container", "kill", "--force"],
            "unknown argument --force",
        ),
        (
            vec!["container", "kill", "tab-name", "--created-by", POCKET],
            "--created-by takes a ref (C followed by digits), not the name 'tab-name'",
        ),
    ] {
        let output = run(&args, &ctx);
        assert_eq!(output.exit_code, 1, "{args:?}: {output:?}");
        assert!(output.stderr.contains(expected), "{args:?}: {output:?}");
    }
    assert!(!log.exists(), "no kill script ran");
}

#[test]
fn the_help_and_usage_name_the_machine_flags() {
    let help = run(&["help"], &CliContext::default());
    for needle in [
        "ls [--all] [--json]",
        "kill <ref|name>  End one environment (exit 4 when there is nothing to kill)",
        "kill <ref> --created-by <key>",
        "(exit 3, nothing killed, when another key created it; exit 4 when there is nothing to kill)",
    ] {
        assert!(help.stdout.contains(needle), "help misses {needle}");
    }
    let usage = run(&["container"], &CliContext::default());
    for needle in [
        "usage: quecto container ls [--all] [--json]",
        "usage: quecto container kill <ref|name>\n",
        "usage: quecto container kill <ref> --created-by <key>\n",
    ] {
        assert!(usage.stderr.contains(needle), "usage misses {needle}");
    }
}

#[test]
fn an_unknown_ref_is_nothing_to_kill_with_or_without_a_creator() {
    let (_dir, ctx, log) = fixture();
    for args in [
        vec!["container", "kill", "C9"],
        vec!["container", "kill", "C9", "--created-by", POCKET],
        vec!["container", "kill", "no-such-name"],
    ] {
        let output = run(&args, &ctx);
        assert_eq!(
            output.exit_code, EXIT_NOTHING_TO_KILL,
            "{args:?}: {output:?}"
        );
        assert!(output.stderr.contains("is unknown"), "{output:?}");
    }
    assert!(!log.exists());
    assert_eq!(EXIT_NOTHING_TO_KILL, 4);
}

/// A record the kill's own restore forgot is nothing to kill — once its
/// creator matches; another creator's is refused as such.
#[test]
fn a_record_forgotten_by_the_kills_restore_is_judged_by_its_creator_first() {
    let (_dir, ctx, log) = fixture();
    let output = run(&["container", "kill", "C5", "--created-by", POCKET], &ctx);
    assert_eq!(output.exit_code, EXIT_NOT_CREATED_BY, "{output:?}");
    assert!(
        output.stderr.contains(
            "environment C5 was created by cli:other, not cli:pocket-7; nothing was killed"
        ),
        "{output:?}"
    );
    assert!(!log.exists());
    let (_dir, ctx, _) = fixture();
    let output = run(&["container", "kill", "C4", "--created-by", POCKET], &ctx);
    assert_eq!(output.exit_code, EXIT_NOTHING_TO_KILL, "{output:?}");
    assert!(
        output
            .stderr
            .contains("environment 'C4' is stopped; nothing of it was left, so it was forgotten"),
        "{output:?}"
    );
    let (_dir, ctx, _) = fixture();
    let output = run(&["container", "kill", "C4"], &ctx);
    assert_eq!(output.exit_code, EXIT_NOTHING_TO_KILL, "{output:?}");
}

#[test]
fn an_empty_name_lists_as_null() {
    let (dir, ctx, _) = fixture();
    let store = FileEnvironmentRegistryStore::for_base_dir(&dir.path().join("base"));
    let mut named_empty = store
        .load()
        .unwrap()
        .into_iter()
        .find(|record| record.environment_ref == "C1")
        .unwrap();
    named_empty.environment_ref = "C6".into();
    named_empty.environment_uuid = "uuid-C6".into();
    named_empty.name = Some(String::new());
    store.record(&named_empty).unwrap();
    let listing = parsed(&run(&["container", "ls", "--json"], &ctx).stdout);
    let six = environments(&listing)
        .iter()
        .find(|row| row["ref"] == "C6")
        .expect("C6 listed");
    assert_eq!(six["name"], serde_json::Value::Null);
}

/// A name only stopped environments carry is nothing this command can
/// end, as a stopped ref whose box left nothing is: both exit 4.
#[test]
fn a_stopped_name_is_nothing_to_kill_like_a_stopped_ref() {
    let (dir, ctx, log) = fixture();
    let store = FileEnvironmentRegistryStore::for_base_dir(&dir.path().join("base"));
    let mut stopped = store
        .load()
        .unwrap()
        .into_iter()
        .find(|record| record.environment_ref == "C3")
        .unwrap();
    stopped.name = Some("old-box".into());
    store.record(&stopped).unwrap();
    let output = run(&["container", "kill", "old-box"], &ctx);
    assert_eq!(output.exit_code, EXIT_NOTHING_TO_KILL, "{output:?}");
    assert!(
        output
            .stderr
            .contains("environment 'old-box' is stopped; nothing live has that name"),
        "{output:?}"
    );
    assert!(!log.exists());
}
