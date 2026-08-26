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
fn selection_reports_a_structured_reason_and_limits_from_the_published_snapshot() {
    let select = flat("src/interface/cli/uds_dispatch_runtime.rs");
    let resolve = select
        .find("ResolveModelSelectionUseCase")
        .expect("selection use case");
    let mutate = select.find("ctx.agent.set_model").expect("agent mutation");
    // The reason is derived from the published snapshot before the session
    // changes, and travels with the response; selecting a model records it
    // rather than refusing, which is the pre-existing UDS contract.
    assert!(resolve < mutate);
    assert!(select.contains("SelectionFailure::Unavailable"));
    assert!(select.contains("\"unavailable\": reason"));
    assert!(select.contains("model_limits_in"));
    assert!(!select.contains("model_limits_from_base_dir"));
}

#[test]
fn startup_and_reload_both_use_the_shared_snapshot_composer() {
    let startup = flat("src/interface/cli/agent.rs");
    let reload = flat("src/interface/cli/provider_reload.rs");
    assert!(startup.contains("build_runtime_snapshot"));
    assert!(reload.contains("build_runtime_snapshot"));
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
fn composition_root_repl_and_uds_share_the_published_snapshot() {
    let composer = flat("src/interface/catalogue_runtime.rs");
    let cli = flat("src/interface/cli/mod.rs");
    let repl = flat("src/interface/repl/mod.rs");
    let agent = flat("src/application/agent_loop.rs");
    let list = flat("src/interface/cli/uds_models.rs");
    let select = flat("src/interface/cli/uds_dispatch_runtime.rs");

    assert!(composer.contains("compose_catalogue_runtime"));
    assert!(cli.contains("build_runtime_snapshot") && cli.contains("runtime,"));
    assert!(repl.contains("ctx.runtime.catalogue") && !repl.contains("model_limits_from_base_dir"));
    assert!(agent.contains("catalogue_store: CatalogueSnapshotStore"));
    assert!(list.contains("agent.catalogue_store.clone()"));
    assert!(select.contains("agent.catalogue_store.current()"));
}

#[test]
fn effective_catalogue_is_resolved_from_ordered_application_owned_layers() {
    let app = flat("src/application/catalogue.rs");
    let runtime = flat("src/application/provider_runtime.rs");
    let composer = flat("src/interface/catalogue_runtime.rs");

    assert!(app.contains("pub trait CatalogueSource"));
    assert!(app.contains("pub fn resolve_sources"));
    assert!(runtime.contains("resolve_sources"));
    assert!(composer.contains("BuiltinCatalogueSource"));
    assert!(composer.contains("UserModelsJsonCatalogueSource"));
    // Precedence belongs to the application: the legacy registry parser must
    // expose the user layer on its own rather than deciding the merge itself.
    let registry = flat("src/infrastructure/model_registry.rs");
    assert!(registry.contains("pub fn load_user_layer_from_path"));
    assert!(!registry.contains("pub fn model_limits_from_base_dir"));
}

#[test]
fn availability_is_exposed_as_distinct_derived_views() {
    let app = flat("src/application/catalogue.rs");
    for view in [
        "CatalogueQuery::Known",
        "CatalogueQuery::Configured",
        "CatalogueQuery::Available",
        "CatalogueQuery::Runnable",
    ] {
        assert!(app.contains(view), "missing derived view: {view}");
    }
    let domain = flat("src/domain/catalogue.rs");
    assert!(domain.contains("pub fn adapter_supported"));
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
        "source precedence",
        "lowest precedence first",
        "application",
        "existing transport",
        "new transport",
        "user override",
    ] {
        assert!(docs.contains(phrase), "missing documentation: {phrase}");
    }
}
