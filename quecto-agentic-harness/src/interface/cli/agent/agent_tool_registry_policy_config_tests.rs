//! #2247 round 2 L1, end to end: the tool-policy command the config runbook
//! documents, and the one a shadowed persist advises, run as written through
//! `quecto config set` against a hermetic base and working directory; the
//! entry then applies to the tool it names when an agent builds its tools.

use std::collections::HashMap;

use tempfile::TempDir;

use super::policy_tests::{build, cli_flags};
use crate::application::configuration::dto::{ConfigLayers, ConfigSelection};
use crate::domain::tool_policy::value_objects::tool::{ToolPolicyApplyMode, ToolPolicyMutation};
use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::Config;
use crate::interface::cli::config_loading::load_selected_config;
use crate::interface::cli::{CliContext, run_with_output};

const RUNBOOK: &str = include_str!("../../../../docs/docs-tool-embeds/config.md");

/// A hermetic `quecto` invocation: its base directory and working directory.
struct Rig {
    base: TempDir,
    cwd: TempDir,
}

impl Rig {
    fn new() -> Self {
        Self {
            base: TempDir::new().unwrap(),
            cwd: TempDir::new().unwrap(),
        }
    }

    fn overlay(&self) -> std::path::PathBuf {
        self.cwd.path().join(".quecto").join("config.json")
    }

    /// Run a shell command line as written (`quecto …`).
    fn run(&self, command_line: &str) -> (i32, String) {
        let ctx = CliContext {
            base_dir: Some(self.base.path().to_path_buf()),
            cwd: Some(self.cwd.path().to_path_buf()),
            configuration: Some(crate::composition::configuration::build_configuration_handles),
            admission: Some(crate::composition::admission::build_admission_handles),
            ..Default::default()
        };
        let argv = shell_words(command_line);
        assert_eq!(argv.first().map(String::as_str), Some("quecto"), "{argv:?}");
        let out = run_with_output(argv, &ctx);
        (out.exit_code, format!("{}{}", out.stdout, out.stderr))
    }

    /// The configuration an agent started in the working directory loads.
    fn load(&self) -> Config {
        let selection = ConfigSelection::Layered(ConfigLayers {
            global: self.base.path().join("config.json"),
            overlay: Some(self.overlay()),
            legacy_local: None,
        });
        load_selected_config(
            crate::composition::configuration::build_configuration_handles,
            self.base.path(),
            &selection,
            false,
            &HashMap::new(),
            false,
        )
        .map(|loaded| loaded.config)
        .unwrap()
    }

    /// The effective scope of `name` in a CLI agent's tools, and its stderr.
    fn scope_of(&self, name: &str) -> (ProfileAvailabilityScope, String) {
        let (built, stderr) = build(&cli_flags(), &self.load());
        let entry = built
            .registry
            .catalogue_entries()
            .into_iter()
            .find(|entry| entry.name == name)
            .unwrap();
        (entry.effective_scope, stderr)
    }
}

/// Split a POSIX shell command line the way a shell would for the commands
/// the docs show: words separated by blanks, a single-quoted run taken
/// verbatim, a `#` word starting a comment.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    for character in line.chars() {
        match (quoted, character) {
            (true, '\'') => quoted = false,
            (true, other) => word.get_or_insert_with(String::new).push(other),
            (false, '\'') => {
                quoted = true;
                word.get_or_insert_with(String::new);
            }
            (false, '#') if word.is_none() => break,
            (false, blank) if blank.is_whitespace() => words.extend(word.take()),
            (false, other) => word.get_or_insert_with(String::new).push(other),
        }
    }
    assert!(!quoted, "unbalanced quote in {line:?}");
    words.extend(word);
    words
}

#[test]
fn shell_words_split_like_a_shell_for_the_documented_forms() {
    assert_eq!(
        shell_words(r#"quecto config set a.b '{"scope":"parent"}'   # note"#),
        vec!["quecto", "config", "set", "a.b", r#"{"scope":"parent"}"#]
    );
    assert_eq!(shell_words("quecto  x ''"), vec!["quecto", "x", ""]);
}

/// The runbook's tool-policy example is a command that works: it writes the
/// entry, the agent's tools start with it applied, and nothing is warned.
#[test]
fn the_documented_policy_command_sets_an_entry_that_applies() {
    let documented = RUNBOOK
        .lines()
        .find(|line| line.starts_with("quecto config set tools.policy.entries."))
        .expect("the config runbook documents a tool-policy command");
    let rig = Rig::new();
    let (before, _) = rig.scope_of("bash");
    assert_eq!(before, ProfileAvailabilityScope::Both);

    let (code, output) = rig.run(documented);
    assert_eq!(code, 0, "{documented}: {output}");

    let (after, stderr) = rig.scope_of("bash");
    assert_eq!(after, ProfileAvailabilityScope::Parent, "{documented}");
    assert!(stderr.is_empty(), "no unknown-id warning: {stderr}");
}

/// A persist the overlay would shadow is refused with a command; that
/// command, run as written, changes the overlay's entry, which then applies.
#[test]
fn the_advised_command_of_a_shadowed_persist_works() {
    let rig = Rig::new();
    let bash = "tool.v1:bundled-native:21:quecto:official-tools:bash";
    let (code, output) = rig.run(&format!(
        r#"quecto config set tools.policy.entries.{bash} '{{"scope":"none"}}'"#
    ));
    assert_eq!(code, 0, "{output}");

    let (mut built, _) = build(&cli_flags(), &rig.load());
    let reconciliation = built.registry.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            bash,
            ProfileAvailabilityScope::Child,
            "test",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    let refusal = crate::infrastructure::config::writer::tool_policy::tool_policy_persistence_for(
        rig.base.path(),
        rig.base.path().join("config.json"),
        Some(rig.overlay()),
    )(&reconciliation)
    .unwrap_err();
    let advised = refusal
        .split('`')
        .find(|span| span.starts_with("quecto config set "))
        .unwrap_or_else(|| panic!("the refusal advises a command: {refusal}"));

    let (code, output) = rig.run(advised);
    assert_eq!(code, 0, "{advised}: {output}");
    let (scope, stderr) = rig.scope_of("bash");
    assert_eq!(scope, ProfileAvailabilityScope::Child, "{advised}");
    assert!(stderr.is_empty(), "{stderr}");
}
