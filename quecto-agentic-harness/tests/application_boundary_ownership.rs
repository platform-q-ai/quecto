use std::fs;

fn source(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[test]
fn cli_and_uds_do_not_construct_concrete_catalogue_or_provider_adapters() {
    for path in [
        "src/interface/cli/models.rs",
        "src/interface/cli/uds_reload.rs",
        "src/interface/cli/agent_provider.rs",
        "src/interface/cli/provider_reload.rs",
    ] {
        let text = source(path);
        for concrete in [
            "ModelsJsonCatalogueRefreshAdapter",
            "InfrastructureProviderRuntimeFactory",
            "ProviderRuntimeInputs",
            "build_agent_provider_with_descriptors",
        ] {
            assert!(
                !text.contains(concrete),
                "interface {path} directly owns concrete adapter {concrete}"
            );
        }
    }
}

#[test]
fn initial_and_reload_provider_composition_share_the_application_boundary() {
    let initial = source("src/interface/cli/agent_provider.rs");
    let reload = source("src/interface/cli/provider_reload.rs");
    let boundary = "ProviderRuntimeApplication";

    assert!(
        initial.contains(boundary),
        "initial composition bypasses {boundary}"
    );
    assert!(
        reload.contains(boundary),
        "reload composition bypasses {boundary}"
    );
}

#[test]
fn cli_and_uds_catalogue_refresh_share_the_application_boundary() {
    let cli = source("src/interface/cli/models.rs");
    let uds = source("src/interface/cli/uds_reload.rs");
    let boundary = "CatalogueRefreshApplication";

    assert!(cli.contains(boundary), "CLI refresh bypasses {boundary}");
    assert!(uds.contains(boundary), "UDS refresh bypasses {boundary}");
}
