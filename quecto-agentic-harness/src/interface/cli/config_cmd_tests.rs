//! `quecto config get|set|trust` over a hermetic base directory and
//! working directory: argument parsing, target selection, presentation.

use crate::interface::cli::{CliContext, run_with_output};
use tempfile::TempDir;

struct Rig {
    _base: TempDir,
    _cwd: TempDir,
    ctx: CliContext,
}

impl Rig {
    fn new() -> Self {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(base.path().to_path_buf()),
            cwd: Some(cwd.path().to_path_buf()),
            configuration: Some(crate::composition::configuration::build_configuration_handles),
            ..Default::default()
        };
        Self {
            _base: base,
            _cwd: cwd,
            ctx,
        }
    }

    fn global(&self) -> std::path::PathBuf {
        self._base.path().join("config.json")
    }

    fn overlay(&self) -> std::path::PathBuf {
        self._cwd.path().join(".quecto").join("config.json")
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let mut argv = vec!["quecto".to_string()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        let out = run_with_output(argv, &self.ctx);
        (out.exit_code, out.stdout, out.stderr)
    }
}

#[test]
fn set_defaults_to_the_overlay_and_get_reads_each_scope() {
    let rig = Rig::new();
    std::fs::write(
        rig.global(),
        r#"{"agents":{"defaults":{"model":"global","effort":"high"}}}"#,
    )
    .unwrap();
    let (code, stdout, stderr) = rig.run(&["config", "set", "agents.defaults.model", "\"local\""]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains(&rig.overlay().display().to_string()),
        "{stdout}"
    );
    assert!(stdout.contains("(created) (trusted)"), "{stdout}");

    let (code, stdout, _) = rig.run(&["config", "get", "agents.defaults"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value, serde_json::json!({"model":"local","effort":"high"}));

    let (_, stdout, _) = rig.run(&["config", "get", "--global", "agents.defaults.model"]);
    assert_eq!(stdout.trim(), "\"global\"");
    let (_, stdout, _) = rig.run(&["config", "get", "--local"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"agents":{"defaults":{"model":"local"}}})
    );
    let (code, _, stderr) = rig.run(&["config", "get", "--effective", "agents.defaults.nope"]);
    assert_eq!(code, 1);
    assert!(
        stderr.contains("`agents.defaults.nope` is not set"),
        "{stderr}"
    );
}

#[test]
fn a_bare_word_value_is_taken_as_a_string_and_json_values_as_json() {
    let rig = Rig::new();
    let (code, _, stderr) = rig.run(&[
        "config",
        "set",
        "--global",
        "agents.defaults.model",
        "gpt-5.5",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, _, stderr) = rig.run(&[
        "config",
        "set",
        "--global",
        "agents.defaults.max_tokens",
        "42",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let (code, _, stderr) = rig.run(&[
        "config",
        "set",
        "--global",
        "agents.defaults.temperature",
        "-1",
    ]);
    assert_eq!(code, 0, "a negative number is a value: {stderr}");
    let (code, _, stderr) = rig.run(&["config", "set", "--global", "--", "custom", "-2"]);
    assert_eq!(code, 0, "-- ends the options: {stderr}");
    let (_, stdout, _) = rig.run(&["config", "get", "--global"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"agents":{"defaults":{"model":"gpt-5.5","max_tokens":42,"temperature":-1}},"custom":-2})
    );
    assert!(
        !rig.overlay().exists(),
        "--global never touches the overlay"
    );
}

#[test]
fn set_refuses_global_only_keys_in_the_overlay_and_invalid_values_anywhere() {
    let rig = Rig::new();
    let (code, _, stderr) = rig.run(&["config", "set", "--local", "admission.directory", "\"/x\""]);
    assert_eq!(code, 1);
    assert!(stderr.contains("`admission` is global-only"), "{stderr}");
    assert!(!rig.overlay().exists());
    let (code, _, stderr) = rig.run(&[
        "config",
        "set",
        "--global",
        "agents.defaults.effort",
        "\"bogus\"",
    ]);
    assert_eq!(code, 1);
    assert!(stderr.contains("invalid effort level"), "{stderr}");
    assert!(!rig.global().exists(), "nothing written on a refused patch");
}

#[test]
fn trust_approves_the_overlay_by_default_or_a_named_file() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.overlay().parent().unwrap()).unwrap();
    std::fs::write(
        rig.overlay(),
        r#"{"agents":{"defaults":{"model":"local"}}}"#,
    )
    .unwrap();
    let (code, stdout, stderr) = rig.run(&["config", "get"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout.trim(),
        "{}",
        "untrusted: nothing of the overlay is effective"
    );
    assert!(stderr.contains("not trusted"), "{stderr}");
    let (code, _, _) = rig.run(&["config", "get", "agents.defaults.model"]);
    assert_eq!(
        code, 1,
        "untrusted: the key is not set in the effective view"
    );

    let (code, stdout, stderr) = rig.run(&["config", "trust"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("trusted "), "{stdout}");
    assert!(stdout.contains("sha256"), "{stdout}");
    let (_, stdout, _) = rig.run(&["config", "get", "agents.defaults.model"]);
    assert_eq!(stdout.trim(), "\"local\"");

    let other = rig._cwd.path().join("elsewhere.json");
    std::fs::write(&other, "{}").unwrap();
    let (code, stdout, _) = rig.run(&["config", "trust", "--path", other.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(stdout.contains("elsewhere.json"));
    let (code, _, stderr) = rig.run(&["config", "trust", "--path"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--path requires a file"), "{stderr}");
}

#[test]
fn usage_errors_name_the_problem() {
    let rig = Rig::new();
    for (args, needle) in [
        (vec!["config"], "usage:"),
        (vec!["config", "bogus"], "usage:"),
        (vec!["config", "get", "a", "b"], "at most one path"),
        (vec!["config", "get", "--global", "--local"], "only one of"),
        (vec!["config", "get", "--nope"], "unknown option --nope"),
        (vec!["config", "set", "only-path"], "a path and a value"),
        (vec!["config", "set", "", "1"], "invalid config path"),
        (vec!["config", "trust", "extra"], "no positional"),
    ] {
        let (code, _, stderr) = rig.run(&args);
        assert_eq!(code, 1, "{args:?}");
        assert!(stderr.contains(needle), "{args:?}: {stderr}");
    }
}

#[test]
fn local_targets_need_an_overlay_location() {
    let rig = Rig::new();
    let explicit = rig._base.path().join("explicit.json");
    std::fs::write(&explicit, "{}").unwrap();
    let explicit = explicit.to_str().unwrap();
    let (code, _, stderr) = rig.run(&["--config", explicit, "config", "set", "a", "1"]);
    assert_eq!(code, 1);
    assert!(
        stderr.contains("--local does not apply with --config"),
        "{stderr}"
    );
    let (code, _, stderr) = rig.run(&["--config", explicit, "config", "get", "--local"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no repo-local overlay applies"), "{stderr}");
    let (code, _, stderr) = rig.run(&[
        "--config", explicit, "config", "set", "--global", "custom", "1",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        std::fs::read_to_string(explicit)
            .unwrap()
            .contains("\"custom\": 1")
    );

    let no_cwd = CliContext {
        cwd: None,
        ..rig.ctx.clone()
    };
    let out = run_with_output(
        vec!["quecto".into(), "config".into(), "trust".into()],
        &no_cwd,
    );
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("working directory is unknown"),
        "{}",
        out.stderr
    );
    let out = run_with_output(
        vec!["quecto".into(), "config".into(), "get".into()],
        &CliContext::default(),
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not composed"));
}

#[test]
fn get_redacts_secrets_unless_shown_and_says_so_on_stderr() {
    let rig = Rig::new();
    std::fs::write(
        rig.global(),
        r#"{"providers":{"openai":{"api_key":"sk-1","api_base":"https://x"},"other":{"token":"t"}}}"#,
    )
    .unwrap();
    let (code, stdout, stderr) = rig.run(&["config", "get", "--global"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        !stdout.contains("sk-1") && !stdout.contains("\"t\""),
        "{stdout}"
    );
    assert_eq!(stdout.matches("<redacted>").count(), 2, "{stdout}");
    assert!(stdout.contains("https://x"), "{stdout}");
    assert!(
        stderr.contains(
            "2 secret values printed as \"<redacted>\"; pass --show-secrets to print them"
        ),
        "{stderr}"
    );

    let (code, stdout, stderr) = rig.run(&["config", "get", "providers.openai.api_key"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "\"<redacted>\"\n");
    assert!(stderr.contains("1 secret value printed"), "{stderr}");

    let (code, stdout, stderr) = rig.run(&["config", "get", "--show-secrets", "--global"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("sk-1"), "{stdout}");
    assert!(!stderr.contains("redacted"), "{stderr}");

    // The flag belongs to `get` only.
    let (code, _, stderr) = rig.run(&["config", "set", "--show-secrets", "a", "1"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown option --show-secrets"), "{stderr}");
    let (code, _, stderr) = rig.run(&["config", "trust", "--show-secrets"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown option --show-secrets"), "{stderr}");
}
