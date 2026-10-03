//! #2414 review H1: a configuration that still sets keys the watermark
//! context removed. Every read names every removed key it sets, the file
//! and the exact command that removes each; and each key can be unset on
//! its own, one at a time, though the others are still there.

use crate::interface::cli::{CliContext, run_with_output};
use tempfile::TempDir;

const TOOL_DIAL: &str = "agents.defaults.context_collapse_after_tool_calls";
const MESSAGE_DIAL: &str = "agents.defaults.context_collapse_after_messages";

struct Rig {
    base: TempDir,
    cwd: TempDir,
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
            admission: Some(crate::composition::admission::build_admission_handles),
            ..Default::default()
        };
        Self { base, cwd, ctx }
    }

    fn global(&self) -> std::path::PathBuf {
        self.base.path().join("config.json")
    }

    fn overlay(&self) -> std::path::PathBuf {
        self.cwd.path().join(".quecto").join("config.json")
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let mut argv = vec!["quecto".to_string()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        let out = run_with_output(argv, &self.ctx);
        (out.exit_code, out.stdout, out.stderr)
    }
}

/// The owner's real configuration: both count dials at 100, and a model.
const OWNERS: &str = r#"{"agents":{"defaults":{"model":"gpt-6.1-sol","context_collapse_after_tool_calls":100,"context_collapse_after_messages":100}}}"#;

#[test]
fn config_get_names_every_removed_key_the_file_and_the_command_for_each() {
    let rig = Rig::new();
    std::fs::write(rig.global(), OWNERS).unwrap();
    let (code, _, stderr) = rig.run(&["config", "get"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains(&rig.global().display().to_string()),
        "names the file: {stderr}"
    );
    for key in [TOOL_DIAL, MESSAGE_DIAL] {
        assert!(
            stderr.contains(&format!("quecto config unset {key} --global")),
            "names {key} and the command that removes it: {stderr}"
        );
    }
}

#[test]
fn each_removed_key_can_be_unset_on_its_own() {
    let rig = Rig::new();
    std::fs::write(rig.global(), OWNERS).unwrap();
    let (code, _, stderr) = rig.run(&["config", "unset", TOOL_DIAL, "--global"]);
    assert_eq!(
        code, 0,
        "the first unset, the other key still set: {stderr}"
    );
    let (code, _, stderr) = rig.run(&["config", "unset", MESSAGE_DIAL, "--global"]);
    assert_eq!(code, 0, "the second unset: {stderr}");
    let (code, stdout, stderr) = rig.run(&["config", "get", "agents.defaults.model"]);
    assert_eq!(code, 0, "the configuration loads again: {stderr}");
    assert_eq!(stdout.trim(), "\"gpt-6.1-sol\"");
}

/// The repair never lets a removed key in: a write that sets one is
/// refused, naming it, even while others are already there.
#[test]
fn a_write_that_sets_a_removed_key_is_refused() {
    let rig = Rig::new();
    std::fs::write(rig.global(), OWNERS).unwrap();
    let (code, _, stderr) = rig.run(&[
        "config",
        "set",
        "agents.defaults.context_mode",
        "\"watermark\"",
        "--global",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("context_mode"), "{stderr}");
    let (code, _, stderr) = rig.run(&["config", "unset", TOOL_DIAL, "--global"]);
    assert_eq!(code, 0, "{stderr}");
}

/// An overlay trusted before the upgrade, now carrying `removed` keys
/// (beside a model): the trust record holds its exact bytes.
fn trusted_overlay(rig: &Rig, removed: &[&str]) {
    use crate::application::configuration::ports::OverlayTrustStore;
    let mut defaults = serde_json::json!({"model": "local"});
    for key in removed {
        defaults[*key] = 100.into();
    }
    let overlay = serde_json::json!({"agents": {"defaults": defaults}});
    let bytes = overlay.to_string().into_bytes();
    std::fs::create_dir_all(rig.overlay().parent().unwrap()).unwrap();
    std::fs::write(rig.overlay(), &bytes).unwrap();
    crate::infrastructure::config::persistence::PersistentOverlayTrustStore::for_base_dir(
        rig.base.path(),
        false,
    )
    .approve(&rig.overlay(), &bytes)
    .unwrap();
}

/// Review round 2 L2a: removed keys in the global file and in a trusted
/// overlay are named in one message, each with its own layer's command.
#[test]
fn removed_keys_in_both_layers_are_named_in_one_message() {
    let rig = Rig::new();
    std::fs::write(rig.global(), OWNERS).unwrap();
    trusted_overlay(&rig, &["context_mode"]);
    let (code, _, stderr) = rig.run(&["config", "get"]);
    assert_eq!(code, 1, "{stderr}");
    for (key, layer) in [
        (TOOL_DIAL, "--global"),
        (MESSAGE_DIAL, "--global"),
        ("agents.defaults.context_mode", "--local"),
    ] {
        assert!(
            stderr.contains(&format!("quecto config unset {key} {layer}")),
            "{key} {layer}: {stderr}"
        );
    }
    assert!(
        stderr.contains(&rig.overlay().display().to_string()),
        "names the overlay: {stderr}"
    );
}

/// Review round 2 L2b: a removed key in a trusted overlay is unset with
/// `--local`, and the configuration loads again.
#[test]
fn a_removed_key_in_a_trusted_overlay_is_unset_with_local() {
    let rig = Rig::new();
    trusted_overlay(&rig, &[MESSAGE_DIAL.trim_start_matches("agents.defaults.")]);
    let (code, _, stderr) = rig.run(&["config", "get"]);
    assert_eq!(code, 1, "refused while the key is there: {stderr}");
    assert!(
        stderr.contains(&format!("quecto config unset {MESSAGE_DIAL} --local")),
        "{stderr}"
    );
    let (code, _, stderr) = rig.run(&["config", "unset", MESSAGE_DIAL, "--local"]);
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, stderr) = rig.run(&["config", "get", "agents.defaults.model"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.trim(), "\"local\"");
}

/// Review round 2 M1: an untrusted overlay cannot be written (a write
/// records trust), so its advice is to edit the file, then trust it; and
/// `config trust` refuses it with the same workable advice, never a
/// command that is itself refused.
#[test]
fn a_removed_key_in_an_untrusted_overlay_is_edited_out_then_trusted() {
    let rig = Rig::new();
    let (code, _, stderr) = rig.run(&["config", "set", "agents.defaults.model", "\"local\""]);
    assert_eq!(code, 0, "{stderr}");
    let mut overlay: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.overlay()).unwrap()).unwrap();
    overlay["agents"]["defaults"]["context_collapse_after_messages"] = 100.into();
    std::fs::write(rig.overlay(), overlay.to_string()).unwrap();
    let edit = format!(
        "remove these keys from {} by editing it, then run `quecto config trust`",
        rig.overlay().display()
    );
    // Hand-edited, the overlay is no longer trusted: it is not applied.
    let (_, _, stderr) = rig.run(&["config", "get"]);
    assert!(stderr.contains(&edit), "{stderr}");
    assert!(stderr.contains(MESSAGE_DIAL), "{stderr}");
    assert!(!stderr.contains("quecto config unset"), "{stderr}");
    let (code, _, stderr) = rig.run(&["config", "trust"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains(&edit), "{stderr}");
    assert!(!stderr.contains("quecto config unset"), "{stderr}");
    // Edited out, the overlay is trusted and applies.
    overlay["agents"]["defaults"]
        .as_object_mut()
        .unwrap()
        .remove("context_collapse_after_messages");
    std::fs::write(rig.overlay(), overlay.to_string()).unwrap();
    let (code, _, stderr) = rig.run(&["config", "trust"]);
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, stderr) = rig.run(&["config", "get", "agents.defaults.model"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.trim(), "\"local\"");
}
