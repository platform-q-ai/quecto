//! Merge-shape acceptance tests for issue #1193.
//!
//! These tests deliberately combine structural boundary checks with references
//! to the behavioral suites that protect retained production behavior. They
//! avoid broad symbol searches where an unrelated comment/test could mask a
//! stale production declaration.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn compact_rust(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn catalogue_source_port_is_declared_and_layered_resolution_is_production_wired() {
    let source = compact_rust(&read("src/application/catalogue.rs"));
    let runtime = compact_rust(&read("src/application/provider_runtime.rs"));
    let composer = compact_rust(&read("src/interface/catalogue_runtime.rs"));
    let adapters = compact_rust(&read("src/infrastructure/catalogue_registry.rs"));

    assert!(source.contains("pub trait CatalogueSource"));
    assert!(source.contains("pub struct ResolveCatalogueUseCase"));
    // The port must not be an unused seam: production composition resolves
    // ordered layers through it, and infrastructure supplies those layers.
    assert!(runtime.contains("resolve_sources"));
    assert!(composer.contains("BuiltinCatalogueSource") && composer.contains("UserModelsJson"));
    assert!(adapters.contains("impl CatalogueSource for BuiltinCatalogueSource"));
    assert!(adapters.contains("impl CatalogueSource for UserModelsJsonCatalogueSource"));
    assert!(
        adapters.contains("use crate::application::ports::"),
        "infrastructure adapters must reach the application only through its ports module"
    );
}

#[test]
fn catalogue_runtime_module_is_removed_after_atomic_snapshot_moved_to_retained_owner() {
    let module = root().join("src/application/catalogue_runtime.rs");
    assert!(
        !module.exists(),
        "test-only catalogue runtime-composer module remains"
    );

    let application_mod = compact_rust(&read("src/application/mod.rs"));
    let library = compact_rust(&read("src/lib.rs"));
    assert!(
        !application_mod.contains("pub mod catalogue_runtime;"),
        "application still exports the obsolete runtime-composer module"
    );
    assert!(
        !library.contains("catalogue_runtime_app"),
        "library still exposes the obsolete runtime-composer API"
    );
}

#[test]
fn obsolete_catalogue_contract_modules_and_files_are_removed() {
    let contracts = compact_rust(&read("tests/contracts.rs"));
    // The catalogue source port is production-wired, so its contract module is
    // retained; only the abandoned composer contract must stay deleted.
    assert!(contracts.contains("mod catalogue_source;"));
    let obsolete = "catalogue_runtime_composer";
    assert!(
        !contracts.contains(&format!("mod {obsolete};")),
        "obsolete contract remains registered: {obsolete}"
    );
    assert!(
        !root()
            .join(format!("tests/contracts/{obsolete}.rs"))
            .exists(),
        "obsolete contract implementation remains: {obsolete}.rs"
    );
}

#[test]
fn retained_runtime_snapshot_has_one_production_definition() {
    let mut definitions = Vec::new();
    fn visit(path: &Path, definitions: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("read source tree") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                visit(&path, definitions);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.ends_with("_tests.rs") || name == "tests.rs"
                })
                && compact_rust(&fs::read_to_string(&path).expect("read source"))
                    .contains("pub struct CatalogueRuntimeSnapshot")
            {
                definitions.push(path);
            }
        }
    }
    visit(&root().join("src"), &mut definitions);
    assert_eq!(
        definitions.len(),
        1,
        "CatalogueRuntimeSnapshot must have exactly one production definition: {definitions:?}"
    );
}

#[test]
fn reload_api_uses_catalogue_runtime_snapshot_directly_without_compatibility_alias() {
    let provider_reload = compact_rust(&read("src/interface/cli/provider_reload.rs"));
    let uds_reload = compact_rust(&read("src/interface/cli/uds_reload.rs"));

    assert!(
        !provider_reload.contains("type ReloadedProviderRuntime"),
        "redundant compatibility alias remains"
    );
    assert!(
        !provider_reload.contains("ReloadedProviderRuntime"),
        "provider reload signatures still use the compatibility name"
    );
    assert!(
        !uds_reload.contains("ReloadedProviderRuntime"),
        "UDS reload signatures still use the compatibility name"
    );
    assert!(
        provider_reload.contains("CatalogueRuntimeSnapshot"),
        "reload no longer exposes the canonical application snapshot"
    );
}

#[test]
fn production_used_application_seams_remain_declared_and_wired() {
    let provider_app = compact_rust(&read("src/application/provider_runtime.rs"));
    let refresh_app = compact_rust(&read("src/application/catalogue_refresh.rs"));
    let runtime_composer = compact_rust(&read("src/interface/catalogue_runtime.rs"));
    let models = compact_rust(&read("src/interface/cli/models.rs"));
    let uds_reload = compact_rust(&read("src/interface/cli/uds_reload.rs"));
    let dispatch = compact_rust(&read("src/interface/cli/uds_dispatch_runtime.rs"));

    assert!(provider_app.contains("pub struct ComposeProviderRuntimeUseCase"));
    assert!(
        runtime_composer.contains("compose_catalogue_runtime")
            && provider_app.contains("ComposeProviderRuntimeUseCase")
    );
    assert!(refresh_app.contains("pub struct RefreshCatalogueSourceUseCase"));
    assert!(models.contains("RefreshCatalogueSourceUseCase"));
    assert!(uds_reload.contains("RefreshCatalogueSourceUseCase"));
    assert!(
        dispatch.contains("agent .catalogue")
            || dispatch.contains("agent. catalogue")
            || dispatch.contains("agent.catalogue_store")
    );
}

#[test]
fn production_reload_installs_the_composed_snapshot_atomically() {
    let reload = compact_rust(&read("src/interface/cli/uds_reload.rs"));
    let agent_reload = compact_rust(&read("src/application/agent_loop_reload.rs"));

    assert!(
        reload.contains("swap_runtime(runtime)"),
        "production reload does not publish the composed snapshot atomically"
    );
    assert!(
        agent_reload.contains("self.provider = runtime.provider")
            && agent_reload.contains("self.catalogue_store.publish(runtime.catalogue)"),
        "AgentLoop does not install provider and catalogue from one snapshot"
    );
    assert!(
        !reload.contains("swap_provider("),
        "production reload fell back to provider-only publication"
    );
}

#[test]
fn behavioral_regression_suites_cover_every_retained_seam() {
    let provider_tests = read("src/application/provider_runtime_tests.rs");
    let refresh_tests = read("src/application/catalogue_refresh_tests.rs");
    let reload_tests = read("src/interface/cli/provider_reload_tests.rs");
    let swap_tests = read("src/application/agent_loop_swap_tests.rs");

    for (suite, behavior) in [
        (
            &provider_tests,
            "compose_provider_runtime_use_case_delegates",
        ),
        (&refresh_tests, "refresh_use_case_delegates"),
        (&reload_tests, "reload_generations_are_monotonic"),
        (&reload_tests, "catalogue_matches_runtime_descriptors"),
        (&swap_tests, "swap_runtime"),
    ] {
        assert!(
            suite.contains(behavior),
            "retained behavior lacks its focused regression test: {behavior}"
        );
    }
}

#[test]
fn deferred_descriptor_and_provider_runtime_work_stays_out_of_this_pr_cleanup() {
    let descriptor = compact_rust(&read("src/domain/catalogue.rs"));
    for retained_field in [
        "pub base_url: Option<String>",
        "pub auth_header: bool",
        "pub allow_remote_http: bool",
    ] {
        assert!(
            descriptor.contains(retained_field),
            "deferred descriptor redesign leaked into merge cleanup: {retained_field}"
        );
    }
    assert!(
        root()
            .join("src/infrastructure/provider_runtime.rs")
            .exists(),
        "deferred provider-runtime decomposition leaked into merge cleanup"
    );
}
