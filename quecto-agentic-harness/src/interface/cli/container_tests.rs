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

/// A base dir whose global file names one default config whose create
/// script answers `--preflight-only` with `lines`.
fn composed(lines: &str) -> (tempfile::TempDir, CliContext) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().join("base");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let create = script(
        dir.path(),
        "create.sh",
        &format!("[ \"${{@: -1}}\" = --preflight-only ] || exit 9\nprintf '{lines}'\nexit 1"),
    );
    std::fs::write(
        base.join("config.json"),
        serde_json::json!({"container_configs": {"box": {
            "default": true, "create": ["bash", create, "--state-dir", "/s"], "cleanup": ["/bin/true"]
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
        "ok\\truntime-cli\\tpodman at /usr/bin/podman\\t\\nwarn\\tgh\\tgh is not on PATH\\tinstall gh\\nfail\\timage\\timage quecto-box:local is not present\\tbuild it (podman build -t quecto-box:local .)\\n",
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
            "  ✗ image        image quecto-box:local is not present\n    remedy: build it (podman build -t quecto-box:local .)\n"
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
