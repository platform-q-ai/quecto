use super::*;
use crate::application::catalogue::dto::ModelListingOutcome;

#[test]
fn composed_handles_list_the_real_catalogue_of_a_base_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"acme":{"api":"openai-completions","baseUrl":"https://acme.test/v1","apiKey":"$ACME_KEY","models":[{"id":"acme-1"}]}}}"#,
    )
    .unwrap();
    let handles = build_catalogue_handles(tmp.path(), None);
    let before = snapshot_store_for(tmp.path()).current().generation();
    let ModelListingOutcome::Listed(listing) = handles.list_models.list() else {
        panic!("a valid models.json lists");
    };
    // The handles publish into the directory's shared store, not a private
    // one: the generation every other reader of `tmp` sees advanced.
    assert_eq!(
        snapshot_store_for(tmp.path()).current().generation(),
        before + 1
    );
    assert_eq!(listing.generation, before + 1);
    assert!(
        listing
            .models
            .iter()
            .any(|m| m.entry.reference().qualified_id() == "acme/acme-1"),
        "{:?}",
        listing
            .models
            .iter()
            .map(|m| m.entry.reference().qualified_id())
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_handles_and_the_runtime_inputs_describe_themselves_without_the_client_or_builders() {
    let tmp = tempfile::TempDir::new().unwrap();
    let inputs = crate::interface::cli::catalogue_handles::RuntimeConfigurationInputs {
        selection: crate::application::configuration::dto::ConfigSelection::Explicit(
            tmp.path().join("config.json"),
        ),
        inherited_child: false,
        env_overrides: std::collections::HashMap::from([("K".to_string(), "v".to_string())]),
        http_client: reqwest::Client::new(),
        provider_runtime: crate::composition::runtime::build_agent_provider,
        configuration: crate::composition::configuration::build_configuration_handles,
    };
    let rendered = format!("{inputs:?}");
    assert!(
        rendered.starts_with("RuntimeConfigurationInputs { selection: Explicit("),
        "{rendered}"
    );
    assert!(
        rendered.contains(r#"env_overrides: {"K": "v"}, .. }"#),
        "{rendered}"
    );
    let handles = build_catalogue_handles(tmp.path(), Some(&inputs));
    let rendered = format!("{handles:?}");
    assert!(
        rendered.starts_with("CatalogueHandles { list_models: "),
        "{rendered}"
    );
    for field in ["effort", "model", "refresh", "reload"] {
        assert!(
            rendered.contains(&format!(", {field}: ")),
            "{field}: {rendered}"
        );
    }
}
