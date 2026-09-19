use super::*;
use crate::interface::cli::CliOutput;

fn run(args: &[&str], ctx: &CliContext) -> CliOutput {
    let mut argv = vec!["quecto".to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    crate::interface::cli::run_with_output(argv, ctx)
}

/// A script run through `bash` (a file executed directly by a
/// multi-threaded test process can race a concurrent fork, ETXTBSY).
fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    path.to_string_lossy().into_owned()
}

/// The `ok` lines of every check the adapter requires of a successful
/// report, as a `printf` body.
const MANDATORY_OK_LINES: &str = "ok\\tjq\\tjq at /usr/bin/jq\\t\\nok\\tgit\\tgit at /usr/bin/git\\t\\nok\\trepo\\tnot needed\\t\\nok\\tstate-dir\\tstate dir /s is writable\\t\\n";

/// A base dir whose global file names one default config whose create
/// script answers `--preflight-only` with the mandatory checks then
/// `lines`, exiting the way the shipped scripts do (1 after a failed
/// check, else 0); an empty `lines` is a script that prints nothing and
/// exits 1.
fn composed(lines: &str) -> (tempfile::TempDir, CliContext) {
    let (body, exit) = if lines.is_empty() {
        (String::new(), 1)
    } else {
        (
            format!("{MANDATORY_OK_LINES}{lines}"),
            i32::from(lines.contains("fail\\t")),
        )
    };
    composed_with_script(
        &format!("[ \"${{@: -1}}\" = --preflight-only ] || exit 9\nprintf '{body}'\nexit {exit}"),
        &["--state-dir", "/s"],
    )
}

fn composed_with_script(script_body: &str, extra_argv: &[&str]) -> (tempfile::TempDir, CliContext) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().join("base");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let create = script(dir.path(), "create.sh", script_body);
    let mut argv = vec!["bash".to_string(), create];
    argv.extend(extra_argv.iter().map(|s| s.to_string()));
    std::fs::write(
        base.join("config.json"),
        serde_json::json!({"container_configs": {"box": {
            "default": true, "create": argv, "cleanup": ["/bin/true"]
        }}})
        .to_string(),
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(base),
        cwd: Some(cwd),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_doctor: Some(crate::composition::environments::build_container_doctor),
        ..Default::default()
    };
    (dir, ctx)
}

#[test]
fn doctor_presents_each_check_with_its_remedy_and_exits_one_on_a_failure() {
    let (_dir, ctx) = composed(
        "ok\\truntime-cli\\tpodman at /usr/bin/podman\\t\\nwarn\\tgh\\tgh is not on PATH\\tinstall gh\\nfail\\timage\\timage quecto-dev:local is not present\\tbuild it (podman build -t quecto-dev:local .)\\n",
    );
    let output = run(&["container", "doctor"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output
            .stdout
            .starts_with("container config \"box\" (create: "),
        "{}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("  ✓ runtime-cli  podman at /usr/bin/podman\n"),
        "{}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("  ! gh           gh is not on PATH\n    remedy: install gh\n"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains(
            "  ✗ image        image quecto-dev:local is not present\n    remedy: build it (podman build -t quecto-dev:local .)\n"
        ),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.ends_with("1 check failed, 1 warning\n"),
        "{}",
        output.stdout
    );
    assert!(output.stderr.is_empty(), "{}", output.stderr);
}

#[test]
fn doctor_exits_zero_when_nothing_failed_and_prints_no_remedy_for_a_pass() {
    let (_dir, ctx) = composed("ok\\timage\\tpresent\\tnever shown\\n");
    let output = run(&["container", "doctor", "--name", "box"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(!output.stdout.contains("remedy"), "{}", output.stdout);
    assert!(
        output.stdout.ends_with("0 checks failed, 0 warnings\n"),
        "{}",
        output.stdout
    );
}

#[test]
fn doctor_exits_one_with_the_script_s_last_words_when_it_dies_mid_report() {
    let (_dir, ctx) = composed_with_script(
        "printf 'ok\\truntime-cli\\tpodman\\t\\nok\\tjq\\tjq\\t\\n'\necho 'usage: QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)' >&2\nexit 2",
        &[],
    );
    let output = run(&["container", "doctor"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("exited exit status: 2 after 2 checks without reporting a failure: usage: QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)"),
        "{}",
        output.stderr
    );
}

#[test]
fn doctor_never_prints_a_token_embedded_in_the_repo_url() {
    let (_dir, ctx) = composed_with_script(
        &format!(
            "printf '{MANDATORY_OK_LINES}fail\\trepo\\t--repo %s is unreachable\\tcheck the URL\\n' \"$2\"\necho \"create: $2\" >&2\nexit 1"
        ),
        &["--repo", "https://user:ghp_secret@host/x/y"],
    );
    let output = run(&["container", "doctor"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    let combined = format!("{}{}", output.stdout, output.stderr);
    assert!(!combined.contains("ghp_secret"), "{combined}");
    assert!(
        output.stdout.contains("--repo https://***@host/x/y)\n")
            && output
                .stdout
                .contains("--repo https://***@host/x/y is unreachable"),
        "{}",
        output.stdout
    );
}

#[test]
fn doctor_usage_errors_name_the_problem() {
    let (_dir, ctx) = composed("ok\\tx\\ty\\t\\n");
    for (args, needle) in [
        (vec!["container"], "usage: quecto container doctor"),
        (
            vec!["container", "prognosis"],
            "usage: quecto container doctor",
        ),
        (
            vec!["container", "doctor", "--name"],
            "--name requires a container config name",
        ),
        (
            vec!["container", "doctor", "--name", ""],
            "--name requires a container config name",
        ),
        (
            vec!["container", "doctor", "--name", "a", "--name", "b"],
            "--name may be given once",
        ),
        (
            vec!["container", "doctor", "--verbose"],
            "unknown argument --verbose",
        ),
        (
            vec!["container", "doctor", "--name", "nope"],
            "unknown container config 'nope'",
        ),
    ] {
        let output = run(&args, &ctx);
        assert_eq!(output.exit_code, 1, "{args:?}: {output:?}");
        assert!(
            output.stderr.contains(needle),
            "{args:?}: {}",
            output.stderr
        );
        assert!(output.stdout.is_empty(), "{args:?}: {}", output.stdout);
    }
}

#[test]
fn doctor_refuses_without_the_composed_builder() {
    let (_dir, ctx) = composed("ok\\tx\\ty\\t\\n");
    let ctx = CliContext {
        container_doctor: None,
        ..ctx
    };
    let output = run(&["container", "doctor"], &ctx);
    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stderr, "container doctor not composed\n");
}

#[test]
fn doctor_reports_a_script_that_cannot_answer() {
    let (_dir, ctx) = composed("");
    let output = run(&["container", "doctor"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output
            .stderr
            .starts_with("container config 'box': create script `bash "),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains(
            "does not support --preflight-only or reported no checks (exit exit status: 1)"
        ),
        "{}",
        output.stderr
    );
}
