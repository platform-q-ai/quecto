//! #1942 ledger row 12: every production agent construction, the plain CLI
//! run, the UDS agent and a spawned child of either, builds its web_fetch
//! tool through `build_agent_from_config`, which hands the web-fetch
//! factory `interface::shared::web_fetch_client_recipe()`. A recording
//! factory (the real composition underneath) shows the recipe each got, and
//! the built agent is shown to carry the very tool the factory returned,
//! which fetches from a local listener. (The recording factory builds the
//! loopback-admitting graph so that local fetch can succeed; the recipe is
//! the one the entry point passed.)
use super::build_tests::selection_for_test;
use super::workflow_spec_tests::uds_workflow_flags;
use super::*;
use std::sync::Mutex;

type SharedTool = std::sync::Arc<dyn crate::application::tools::ports::Tool>;

/// For each call of the recording factory: the connect timeout of the
/// recipe it was handed, and the address of the tool it returned.
static RECEIVED: Mutex<Vec<(Option<std::time::Duration>, usize)>> = Mutex::new(Vec::new());

/// The address of the tool behind `tool`, to recognise the same instance.
fn identity(tool: &SharedTool) -> usize {
    std::sync::Arc::as_ptr(tool) as *const () as usize
}

fn recording_factory(
    recipe: crate::infrastructure::http::web_fetch::WebFetchClientRecipe,
    max_response_kb: u32,
) -> SharedTool {
    let timeout = recipe.configured_connect_timeout();
    let tool =
        crate::composition::web_fetch::build_allowing_loopback_for_tests(recipe, max_response_kb);
    RECEIVED.lock().unwrap().push((timeout, identity(&tool)));
    tool
}

/// Fetches `http://web-fetch.test:<port>/` through `tool` from a local
/// listener answering `ok`, returning the tool's result content.
fn fetch_through(tool: &SharedTool) -> String {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok")
                .await;
        });
        let arguments = format!(r#"{{"url":"http://web-fetch.test:{port}/","raw":true}}"#);
        let result = tool.execute(&arguments).await.expect("the tool runs");
        peer.abort();
        assert!(!result.is_error, "{}", result.content);
        result.content
    })
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
        let built = built.unwrap_or_else(|| panic!("{entry}: {stderr}"));
        let received = RECEIVED.lock().unwrap().clone();
        assert_eq!(received.len(), before + 1, "{entry}: the factory ran once");
        let (timeout, returned) = *received.last().unwrap();
        assert_eq!(
            timeout,
            crate::interface::shared::web_fetch_client_recipe().configured_connect_timeout(),
            "{entry}: the production recipe"
        );
        assert_eq!(
            timeout,
            Some(crate::infrastructure::providers::CONNECT_TIMEOUT),
            "{entry}"
        );
        // The agent's own registry offers web_fetch ...
        assert!(
            built
                .agent
                .tool_descriptors()
                .iter()
                .any(|descriptor| descriptor.name() == "web_fetch"),
            "{entry}: web_fetch is in the agent's tool registry"
        );
        // ... and it is the tool the factory returned, not another one.
        let registered: Vec<SharedTool> = built
            .ext_registry
            .lock()
            .unwrap()
            .all_tools()
            .into_iter()
            .filter(|tool| tool.definition().name == "web_fetch")
            .collect();
        assert_eq!(registered.len(), 1, "{entry}");
        assert_eq!(
            identity(&registered[0]),
            returned,
            "{entry}: the factory's tool"
        );
        assert_eq!(fetch_through(&registered[0]), "ok", "{entry}: it fetches");
    }
}
