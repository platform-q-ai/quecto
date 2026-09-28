//! #2217: persisted `tools.policy` entries that match no registered tool.

use super::*;
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolSource};
use crate::domain::tool_policy_catalogue::{BUNDLED_TOOLS, bundled_stable_id};
use crate::infrastructure::config::ToolPolicyEntryConfig;
use crate::interface::cli::agent::flag_parse::AgentFlags;

/// Build the tool registry for `flags` and `config`; returns it with stderr.
pub(super) fn build(flags: &AgentFlags, config: &Config) -> (ToolRegistryBuild, String) {
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

pub(super) fn cli_flags() -> AgentFlags {
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

/// Every boolean config switch that gates a bundled tool's registration,
/// as a pointer into the configuration document (#2247 round 2 L3).
const REGISTRATION_SWITCHES: &[&str] = &[
    "/tools/web/brave/enabled",
    "/tools/web/duckduckgo/enabled",
    "/tools/web/fetch/enabled",
];

/// Every other boolean `enabled` switch: it tunes a tool that is always
/// registered (or something that is not a tool), so the drift test need not
/// turn it on. (`workflow`, the one flag-gated tool, is built by
/// `uds_workflow_flags`.)
const NON_REGISTRATION_SWITCHES: &[&str] = &[
    // The session event log: no tool.
    "/telemetry/event_log/enabled",
    // grep's search log and relevance ranking: grep is always registered.
    "/tools/grep/log/enabled",
    "/tools/grep/relevance/enabled",
];

/// Every `.../enabled` boolean in `value`, as a JSON pointer.
fn enabled_switches(value: &serde_json::Value, pointer: &str, found: &mut Vec<String>) {
    if let serde_json::Value::Object(map) = value {
        for (key, child) in map {
            let child_pointer = format!("{pointer}/{key}");
            match (key.as_str(), child) {
                ("enabled", serde_json::Value::Bool(_)) => found.push(child_pointer),
                _ => enabled_switches(child, &child_pointer, found),
            }
        }
    }
}

/// A new `enabled` switch fails here until it is classified: a switch that
/// gates a registration goes in [`REGISTRATION_SWITCHES`], which
/// [`every_optional_tool_enabled`] turns on, so the drift test builds the
/// tool it gates.
#[test]
fn every_enabled_switch_is_classified() {
    let document = serde_json::to_value(Config::default()).unwrap();
    let mut found = Vec::new();
    enabled_switches(&document, "", &mut found);
    found.sort();
    let mut classified: Vec<String> = REGISTRATION_SWITCHES
        .iter()
        .chain(NON_REGISTRATION_SWITCHES)
        .map(|pointer| (*pointer).to_string())
        .collect();
    classified.sort();
    assert_eq!(
        found, classified,
        "classify each config `enabled` switch as gating a tool registration or not"
    );
}

/// The configuration with every [`REGISTRATION_SWITCHES`] switch on: every
/// optional bundled tool is built.
fn every_optional_tool_enabled() -> Config {
    let mut document = serde_json::to_value(Config::default()).unwrap();
    for pointer in REGISTRATION_SWITCHES {
        let switch = document
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("{pointer} is in the default configuration"));
        *switch = serde_json::Value::Bool(true);
    }
    let config = Config::from_document(document).unwrap();
    assert!(config.tools.web.fetch.enabled && config.tools.web.brave.enabled);
    config
}

/// The catalogue cannot drift: every bundled registration on every
/// entrypoint (the one-shot CLI and the UDS agent the TUI drives), with every
/// config-gated tool built, is listed under the id it registers with.
#[test]
fn every_bundled_tool_on_every_entrypoint_is_in_the_catalogue() {
    let config = every_optional_tool_enabled();
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

/// A UDS extension's or a runtime tool's entry applies when that tool
/// registers, after start-up: no warning on any entrypoint (#2247 round 2).
#[test]
fn an_extension_or_runtime_id_prints_no_warning() {
    let config = config_with_policy(&[
        "tool.v1:uds:12:uds:client-a:weather",
        "tool.v1:runtime:15:com.example.ext:fetch-page",
    ]);
    for flags in [cli_flags(), uds_workflow_flags()] {
        let (_, stderr) = build(&flags, &config);
        assert!(stderr.is_empty(), "stderr: {stderr}");
    }
}

/// An entry that is not a stable id at all (a bare name, a malformed id)
/// never matches a registered tool's entry: warned about like a typo.
#[test]
fn a_non_stable_id_is_warned_about() {
    let config = config_with_policy(&["tool.v1:uds:3:uds:client-a:weather"]);
    let (_, stderr) = build(&cli_flags(), &config);
    assert_eq!(
        stderr,
        "WARNING: tools.policy: no tool has stable id 'tool.v1:uds:3:uds:client-a:weather', so its entry never applies; fix or remove it under tools.policy.entries\n"
    );
}
