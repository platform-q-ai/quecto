//! End-to-end architectural acceptance guards for issue #1193's remaining gaps.
use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap()
}
fn flat(path: &str) -> String {
    read(path).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn application_resolution_query_and_selection_are_production_wired() {
    let app = flat("src/application/catalogue.rs");
    let runtime = flat("src/application/provider_runtime.rs");
    let list = flat("src/interface/cli/uds_models.rs");
    let select = flat("src/interface/cli/uds_dispatch_runtime.rs");
    assert!(app.contains("ResolveCatalogueUseCase"));
    assert!(runtime.contains("ResolveCatalogueUseCase"));
    assert!(list.contains("QueryCatalogueUseCase"));
    assert!(select.contains("ResolveModelSelectionUseCase"));
}

#[test]
fn selection_rejects_unknown_and_unavailable_before_mutation() {
    let select = flat("src/interface/cli/uds_dispatch_runtime.rs");
    let resolve = select
        .find("ResolveModelSelectionUseCase")
        .expect("selection use case");
    let mutate = select.find("ctx.agent.set_model").expect("agent mutation");
    assert!(resolve < mutate);
    assert!(select.contains("SelectionFailure::UnknownModel"));
    assert!(select.contains("SelectionFailure::Unavailable"));
}

#[test]
fn startup_and_reload_both_use_the_shared_snapshot_composer() {
    let startup = flat("src/interface/cli/agent.rs");
    let reload = flat("src/interface/cli/provider_reload.rs");
    assert!(startup.contains("build_agent_runtime"));
    assert!(reload.contains("build_agent_runtime"));
    assert!(!reload.contains("build_agent_provider_with_descriptors"));
}

#[test]
fn startup_and_reload_do_not_rederive_catalogue_or_limits() {
    let startup = flat("src/interface/cli/agent.rs");
    let reload = flat("src/interface/cli/provider_reload.rs");
    let production_reload = reload
        .split("#[cfg(any(test, feature = \"test-support\"))]")
        .next()
        .unwrap();
    assert!(!startup.contains("model_limits_from_base_dir"));
    assert!(!production_reload.contains("CatalogueSnapshot::new(0, Vec"));
    assert!(!production_reload.contains("model_descriptors()"));
}

#[test]
fn list_models_polls_reload_before_reading_snapshot() {
    let dispatch = flat("src/interface/cli/uds_dispatch.rs");
    assert!(dispatch.contains("AgentCommand::ListModels"));
    let poll = dispatch
        .find("poll_provider_reload_for_ctx")
        .expect("reload poll");
    let query = dispatch
        .find("dispatch_fieldless_command")
        .expect("query dispatch");
    assert!(
        poll < query,
        "list-model reload must occur before query fast path"
    );
}

#[test]
fn partial_refresh_reloads_after_any_success_and_still_reports_failures() {
    let refresh = flat("src/interface/cli/uds_reload.rs");
    assert!(refresh.contains("any_refreshed"));
    assert!(refresh.contains("any_failed"));
    assert!(refresh.contains("if any_refreshed"));
    assert!(refresh.contains("force_provider_reload"));
}

#[test]
fn tui_has_no_hardcoded_model_catalogue() {
    let selector = flat("../quecto-tui/src/components/model_selector.rs");
    assert!(!selector.contains("fn known_models"));
    assert!(!selector.contains("claude-opus"));
    assert!(!selector.contains("gpt-5."));
}

#[test]
fn consumer_contract_and_docs_cover_single_authority() {
    assert!(
        root()
            .join("tests/contracts/catalogue_consumers.rs")
            .exists()
    );
    let docs = read("../docs/runtime-models-providers.md").to_ascii_lowercase();
    for phrase in [
        "effective catalogue",
        "application",
        "existing transport",
        "new transport",
        "user override",
    ] {
        assert!(docs.contains(phrase), "missing documentation: {phrase}");
    }
}
