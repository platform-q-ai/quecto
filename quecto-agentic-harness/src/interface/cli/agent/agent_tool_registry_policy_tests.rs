//! #2217: persisted `tools.policy` entries that match no registered tool.

use super::*;
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolSource};
use crate::domain::tool_policy_catalogue::{BUNDLED_TOOLS, bundled_stable_id};
use crate::infrastructure::config::ToolPolicyEntryConfig;
use crate::interface::cli::agent::flag_parse::AgentFlags;

/// Build the tool registry for `flags` and `config`; returns it with stderr.
fn build(flags: &AgentFlags, config: &Config) -> (ToolRegistryBuild, String) {
    let tmp = tempfile::TempDir::new().unwrap();
    let http = reqwest::Client::new();
    let mut stderr = String::new();
    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        effort_control: crate::composition::catalogue::build_catalogue_handles(
            std::path::Path::new("/nonexistent-catalogue"),
            None,
        )
        .effort,
        container_configs: crate::composition::container_configs::build_container_config_handles(
            std::path::Path::new("/nonexistent-base"),
            None,
        ),
        config_path: tmp.path(),
        config,
        http_client: &http,
        web_fetch_tool: config.tools.web.fetch.enabled.then(|| {
            crate::composition::web_fetch::build(
                crate::infrastructure::http::web_fetch::WebFetchClientRecipe::default(),
                1024,
            )
        }),
        flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();
    (built, stderr)
}

fn cli_flags() -> AgentFlags {
    let mut flags = super::cov_tests::flags();
    flags.no_session = true;
    flags
}

fn uds_workflow_flags() -> AgentFlags {
    let mut flags = cli_flags();
    flags.uds_mode = true;
    flags.workflow_disabled = false;
    flags.workflow = true;
    flags
}

fn config_with_policy(stable_ids: &[&str]) -> Config {
    let mut config = Config::default();
    for stable_id in stable_ids {
        config.tools.policy.entries.insert(
            (*stable_id).to_string(),
            ToolPolicyEntryConfig {
                scope: ProfileAvailabilityScope::Both,
            },
        );
    }
    config
}

/// The catalogue cannot drift: every bundled registration on every
/// entrypoint (the one-shot CLI and the UDS agent the TUI drives), with every
/// config-gated tool built, is listed under the id it registers with.
#[test]
fn every_bundled_tool_on_every_entrypoint_is_in_the_catalogue() {
    let mut config = Config::default();
    config.tools.web.fetch.enabled = true;
    config.tools.web.brave.enabled = true;
    config.tools.web.brave.api_key = "test-key".into();
    let mut registered = std::collections::BTreeSet::new();
    for flags in [cli_flags(), uds_workflow_flags()] {
        let (built, _) = build(&flags, &config);
        for entry in built.registry.catalogue_entries() {
            assert!(
                entry.source == ToolSource::BundledNative,
                "an entrypoint registers only bundled tools: {}",
                entry.stable_id
            );
            let pair = (entry.provider_id.to_string(), entry.name.to_string());
            assert_eq!(entry.stable_id, bundled_stable_id(&pair.0, &pair.1));
            registered.insert(pair);
        }
    }
    let catalogued: std::collections::BTreeSet<_> = BUNDLED_TOOLS
        .iter()
        .map(|(provider_id, name)| (provider_id.to_string(), name.to_string()))
        .collect();
    let missing: Vec<_> = registered.difference(&catalogued).collect();
    assert!(
        missing.is_empty(),
        "bundled but not in BUNDLED_TOOLS: {missing:?}"
    );
    // Every catalogued tool is built somewhere: the list holds no stale entry
    // (a removed tool moves to RETIRED_TOOLS instead).
    let stale: Vec<_> = catalogued.difference(&registered).collect();
    assert!(
        stale.is_empty(),
        "in BUNDLED_TOOLS but built nowhere: {stale:?}"
    );
}

/// A bundled tool this entrypoint does not build (`workflow` on the one-shot
/// CLI) is kept for the others without a start-up warning.
#[test]
fn a_bundled_tool_another_entrypoint_builds_prints_no_warning() {
    let config = config_with_policy(&["tool.v1:bundled-native:15:quecto:workflow:workflow"]);
    let (built, stderr) = build(&cli_flags(), &config);
    assert!(
        built.registry.get("workflow").is_none(),
        "the CLI builds no workflow"
    );
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

/// A retired tool's entry (`python_lab`, #1684) is dead but harmless: no
/// start-up warning.
#[test]
fn a_retired_tool_prints_no_warning() {
    let config =
        config_with_policy(&["tool.v1:bundled-native:21:quecto:official-tools:python_lab"]);
    let (_, stderr) = build(&cli_flags(), &config);
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

/// An id no bundled tool ever had is most likely a typo: a restriction that
/// never applies. It is warned about on stderr, naming the id and where to
/// fix it.
#[test]
fn an_unknown_id_is_warned_about_on_stderr() {
    let typo = "tool.v1:bundled-native:21:quecto:official-tools:bsah";
    let config = config_with_policy(&[typo]);
    for flags in [cli_flags(), uds_workflow_flags()] {
        let (_, stderr) = build(&flags, &config);
        assert_eq!(
            stderr,
            format!(
                "WARNING: tools.policy: no tool has stable id '{typo}', so its entry never applies; fix or remove it under tools.policy.entries\n"
            )
        );
    }
}
