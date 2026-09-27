//! #1942 ledger row 12: every production agent construction, the plain CLI
//! run, the UDS agent and a spawned child of either, builds its web_fetch
//! tool through `build_agent_from_config`, which hands the web-fetch
//! factory `interface::shared::web_fetch_client_recipe()`. A recording
//! factory (the real composition underneath) shows the recipe each got.
use super::build_tests::selection_for_test;
use super::workflow_spec_tests::uds_workflow_flags;
use super::*;
use std::sync::Mutex;

/// The connect timeout of every recipe the recording factory was handed.
static RECEIVED: Mutex<Vec<Option<std::time::Duration>>> = Mutex::new(Vec::new());

fn recording_factory(
    recipe: crate::infrastructure::http::web_fetch::WebFetchClientRecipe,
    max_response_kb: u32,
) -> std::sync::Arc<dyn crate::application::tools::ports::Tool> {
    RECEIVED
        .lock()
        .unwrap()
        .push(recipe.configured_connect_timeout());
    crate::composition::web_fetch::build(recipe, max_response_kb)
}

#[test]
fn every_agent_entry_point_builds_web_fetch_from_the_production_recipe() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}},"tools":{"web":{"fetch":{"enabled":true}}}}"#,
    )
    .unwrap();
    let cfg = tmp.path().join("config.json");
    for (entry, uds_mode, spawned) in [
        ("cli", false, false),
        ("uds", true, false),
        ("spawned cli child", false, true),
        ("spawned uds child", true, true),
    ] {
        let mut flags = uds_workflow_flags(false, false);
        flags.uds_mode = uds_mode;
        flags.spawned = spawned;
        flags.message = (!uds_mode).then(|| "hi".to_owned());
        flags.web_fetch_tool_factory = Some(recording_factory);
        let before = RECEIVED.lock().unwrap().len();
        let mut stderr = String::new();
        let built = build_agent_from_config(
            tmp.path(),
            &selection_for_test(&cfg, false),
            &flags,
            &mut stderr,
            None,
        );
        assert!(built.is_some(), "{entry}: {stderr}");
        let received = RECEIVED.lock().unwrap().clone();
        assert_eq!(received.len(), before + 1, "{entry}: the factory ran once");
        assert_eq!(
            received.last().copied().flatten(),
            crate::interface::shared::web_fetch_client_recipe().configured_connect_timeout(),
            "{entry}: the production recipe"
        );
        assert_eq!(
            received.last().copied().flatten(),
            Some(crate::infrastructure::providers::CONNECT_TIMEOUT),
            "{entry}"
        );
    }
}
