use cucumber::{given, then, when};
use quecto::domain::error::DomainError;
use quecto::domain::extension::Extension;
use quecto::domain::tool::{Tool, ToolDefinition, ToolResult};
use quecto::infrastructure::config::Config;
use quecto::infrastructure::extensions::native::{NativeExtension, build_native_extensions};
use quecto::infrastructure::extensions::registry::ExtensionRegistry;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{DebugExtension, QuectoWorld};

// ─── Simple test extension ───────────────────────────────────────────────────

struct TestExtension {
    name: String,
    tools: Vec<Arc<dyn Tool>>,
    snippet: Option<String>,
}

impl Extension for TestExtension {
    fn name(&self) -> &str {
        &self.name
    }
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }
    fn system_prompt_snippet(&self) -> Option<String> {
        self.snippet.clone()
    }
}

struct DummyTool {
    name: String,
    description: String,
}

impl Tool for DummyTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone().into(),
            description: self.description.clone().into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: "ok".to_string(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

// ─── Extension trait steps ───────────────────────────────────────────────────

#[given(expr = "an extension named {string} with {int} tools")]
fn given_extension_with_tools(world: &mut QuectoWorld, name: String, count: i32) {
    let tools: Vec<Arc<dyn Tool>> = (0..count)
        .map(|i| {
            Arc::new(DummyTool {
                name: format!("tool_{}", i),
                description: format!("Tool {}", i),
            }) as Arc<dyn Tool>
        })
        .collect();
    world.test_extension = Some(DebugExtension(Arc::new(TestExtension {
        name,
        tools,
        snippet: None,
    })));
}

#[given(expr = "an extension named {string} with prompt snippet {string}")]
fn given_extension_with_snippet(world: &mut QuectoWorld, name: String, snippet: String) {
    world.test_extension = Some(DebugExtension(Arc::new(TestExtension {
        name,
        tools: vec![],
        snippet: Some(snippet),
    })));
}

#[then(expr = "the extension name should be {string}")]
fn then_extension_name(world: &mut QuectoWorld, expected: String) {
    let ext = world.test_extension.as_ref().expect("no extension");
    assert_eq!(ext.0.name(), expected);
}

#[then(expr = "the extension should provide {int} tools")]
fn then_extension_tool_count(world: &mut QuectoWorld, expected: i32) {
    let ext = world.test_extension.as_ref().expect("no extension");
    assert_eq!(ext.0.tools().len(), expected as usize);
}

#[then("the extension system prompt snippet should be None")]
fn then_extension_snippet_none(world: &mut QuectoWorld) {
    let ext = world.test_extension.as_ref().expect("no extension");
    assert!(ext.0.system_prompt_snippet().is_none());
}

#[then(expr = "the extension system prompt snippet should be {string}")]
fn then_extension_snippet_is(world: &mut QuectoWorld, expected: String) {
    let ext = world.test_extension.as_ref().expect("no extension");
    assert_eq!(ext.0.system_prompt_snippet().unwrap(), expected);
}

// ─── Extension registry steps ────────────────────────────────────────────────

#[given("an empty extension registry")]
fn given_empty_registry(world: &mut QuectoWorld) {
    world.ext_registry = Some(ExtensionRegistry::new());
}

#[given(expr = "an extension named {string} with {int} tools is registered")]
fn given_ext_registered(world: &mut QuectoWorld, name: String, count: i32) {
    let tools: Vec<Arc<dyn Tool>> = (0..count)
        .map(|i| {
            Arc::new(DummyTool {
                name: format!("{}_{}", name, i),
                description: format!("Tool {} from {}", i, name),
            }) as Arc<dyn Tool>
        })
        .collect();
    let reg = world.ext_registry.as_mut().expect("need registry");
    reg.register(Arc::new(TestExtension {
        name,
        tools,
        snippet: None,
    }));
}

#[given(expr = "an extension with prompt snippet {string} is registered")]
fn given_ext_with_snippet_registered(world: &mut QuectoWorld, snippet: String) {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let reg = world.ext_registry.as_mut().expect("need registry");
    reg.register(Arc::new(TestExtension {
        name: format!("snippet-ext-{}", id),
        tools: vec![],
        snippet: Some(snippet),
    }));
}

#[given(expr = "an extension with tool named {string} and description {string} is registered")]
fn given_ext_with_named_tool(world: &mut QuectoWorld, tool_name: String, desc: String) {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let reg = world.ext_registry.as_mut().expect("need registry");
    reg.register(Arc::new(TestExtension {
        name: format!("ext-{}", id),
        tools: vec![Arc::new(DummyTool {
            name: tool_name,
            description: desc,
        })],
        snippet: None,
    }));
}

#[then(expr = "the registry should have {int} extension tools")]
fn then_registry_tool_count(world: &mut QuectoWorld, expected: i32) {
    let reg = world.ext_registry.as_ref().expect("need registry");
    assert_eq!(reg.all_tools().len(), expected as usize);
}

#[then("the registry system prompt snippets should be empty")]
fn then_registry_snippets_empty(world: &mut QuectoWorld) {
    let reg = world.ext_registry.as_ref().expect("need registry");
    assert!(reg.system_prompt_snippets().is_empty());
}

#[then(expr = "the registry system prompt snippets should contain {string}")]
fn then_registry_snippets_contain(world: &mut QuectoWorld, expected: String) {
    let reg = world.ext_registry.as_ref().expect("need registry");
    let snippets = reg.system_prompt_snippets();
    assert!(
        snippets.contains(&expected),
        "expected snippets to contain '{}', got: {}",
        expected,
        snippets
    );
}

#[then(expr = "the extension tool {string} should have description {string}")]
fn then_tool_has_description(world: &mut QuectoWorld, tool_name: String, expected_desc: String) {
    let reg = world.ext_registry.as_ref().expect("need registry");
    let tools = reg.all_tools();
    let tool = tools
        .iter()
        .find(|t| t.definition().name.as_ref() == tool_name)
        .unwrap_or_else(|| panic!("tool '{}' not found", tool_name));
    assert_eq!(tool.definition().description.as_ref(), expected_desc);
}

#[then(expr = "the registry should contain tool {string}")]
fn then_registry_contains_tool(world: &mut QuectoWorld, name: String) {
    let reg = world.ext_registry.as_ref().expect("need registry");
    let tools = reg.all_tools();
    assert!(
        tools.iter().any(|t| t.definition().name.as_ref() == name),
        "tool '{}' not found in registry",
        name
    );
}

// ─── Native extension steps (#351) ───────────────────────────────────────────

#[given(expr = "a native extension named {string} wrapping a tool with description {string}")]
fn given_native_extension(world: &mut QuectoWorld, name: String, desc: String) {
    let tool: Arc<dyn Tool> = Arc::new(DummyTool {
        name: name.clone(),
        description: desc,
    });
    let ext = NativeExtension::new(name, "native ext", tool);
    world.native_extension = Some(DebugExtension(Arc::new(ext)));
}

#[given(expr = "a native extension named {string} with system prompt {string}")]
fn given_native_extension_with_prompt(world: &mut QuectoWorld, name: String, prompt: String) {
    let tool: Arc<dyn Tool> = Arc::new(DummyTool {
        name: name.clone(),
        description: "test".to_string(),
    });
    let ext = NativeExtension::new(name, "native ext", tool).with_system_prompt(prompt);
    world.native_extension = Some(DebugExtension(Arc::new(ext)));
}

#[then(expr = "the native extension name should be {string}")]
fn then_native_ext_name(world: &mut QuectoWorld, expected: String) {
    let ext = world
        .native_extension
        .as_ref()
        .expect("no native extension");
    assert_eq!(ext.0.name(), expected);
}

#[then(expr = "the native extension should provide {int} tool")]
fn then_native_ext_tool_count(world: &mut QuectoWorld, expected: i32) {
    let ext = world
        .native_extension
        .as_ref()
        .expect("no native extension");
    assert_eq!(ext.0.tools().len(), expected as usize);
}

#[then(expr = "the native extension tool should have name {string}")]
fn then_native_ext_tool_name(world: &mut QuectoWorld, expected: String) {
    let ext = world
        .native_extension
        .as_ref()
        .expect("no native extension");
    let tools = ext.0.tools();
    assert!(
        tools
            .iter()
            .any(|t| t.definition().name.as_ref() == expected),
        "expected tool '{}'",
        expected
    );
}

#[then(expr = "the native extension system prompt snippet should be {string}")]
fn then_native_ext_snippet(world: &mut QuectoWorld, expected: String) {
    let ext = world
        .native_extension
        .as_ref()
        .expect("no native extension");
    assert_eq!(
        ext.0.system_prompt_snippet().as_deref(),
        Some(expected.as_str())
    );
}

#[then("the native extension system prompt snippet should be None")]
fn then_native_ext_snippet_none(world: &mut QuectoWorld) {
    let ext = world
        .native_extension
        .as_ref()
        .expect("no native extension");
    assert!(ext.0.system_prompt_snippet().is_none());
}

#[given(expr = "a native extension named {string} is registered in the extension registry")]
fn given_native_ext_in_registry(world: &mut QuectoWorld, name: String) {
    let tool: Arc<dyn Tool> = Arc::new(DummyTool {
        name: name.clone(),
        description: format!("Native {}", name),
    });
    let ext = Arc::new(NativeExtension::new(
        &name,
        format!("Native {}", name),
        tool,
    ));
    let reg = world.ext_registry.as_mut().expect("need registry");
    reg.register(ext);
}

#[given(expr = "a native extension {string} registered as a bundled native tool")]
#[when(expr = "a native extension {string} is registered as a bundled native tool")]
fn given_native_ext_in_tool_registry(world: &mut QuectoWorld, name: String) {
    let tool: Arc<dyn Tool> = Arc::new(DummyTool {
        name: name.clone(),
        description: format!("Native {}", name),
    });
    let reg = world.tool_registry.as_mut().expect("need tool registry");
    reg.register(tool);
}

#[then(expr = "the tool registry extension names should include {string}")]
fn then_tool_registry_ext_names_include(world: &mut QuectoWorld, name: String) {
    let reg = world.tool_registry.as_ref().expect("need tool registry");
    let names = reg.runtime_tool_names();
    assert!(
        names.contains(&name),
        "extension '{}' not in extension_names: {:?}",
        name,
        names
    );
}

#[then(expr = "the tool registry extension names should not include {string}")]
fn then_tool_registry_ext_names_exclude(world: &mut QuectoWorld, name: String) {
    let reg = world.tool_registry.as_ref().expect("need tool registry");
    let names = reg.runtime_tool_names();
    assert!(
        !names.contains(&name),
        "extension '{}' should not be in extension_names: {:?}",
        name,
        names
    );
}

// ─── Config-driven native extension steps (#351) ─────────────────────────────

fn config_with_web(
    brave_enabled: bool,
    brave_key: &str,
    ddg_enabled: bool,
    fetch_enabled: bool,
) -> Config {
    let mut config = Config::default();
    config.tools.web.brave.enabled = brave_enabled;
    config.tools.web.brave.api_key = brave_key.to_string();
    config.tools.web.duckduckgo.enabled = ddg_enabled;
    config.tools.web.fetch.enabled = fetch_enabled;
    config
}

fn config_with_web_search(brave_enabled: bool, brave_key: &str, ddg_enabled: bool) -> Config {
    config_with_web(brave_enabled, brave_key, ddg_enabled, false)
}

#[given(expr = "a config with tools.web.brave.enabled = true and api_key = {string}")]
fn given_config_brave_enabled(world: &mut QuectoWorld, api_key: String) {
    world.config = Some(config_with_web_search(true, &api_key, false));
}

#[given("a config with tools.web.duckduckgo.enabled = true")]
fn given_config_ddg_enabled(world: &mut QuectoWorld) {
    world.config = Some(config_with_web_search(false, "", true));
}

#[given("a config with tools.web.brave.enabled = false and tools.web.duckduckgo.enabled = false")]
fn given_config_web_disabled(world: &mut QuectoWorld) {
    world.config = Some(config_with_web_search(false, "", false));
}

#[when("I build native extensions from config")]
fn when_build_native_extensions(world: &mut QuectoWorld) {
    let config = world.config.as_ref().expect("no config");
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&config.tools.web, &client);
    world.native_extensions_built = Some(exts.into_iter().map(DebugExtension).collect());
}

#[then(expr = "the native extensions list should contain {string}")]
fn then_native_exts_contain(world: &mut QuectoWorld, name: String) {
    let exts = world
        .native_extensions_built
        .as_ref()
        .expect("no native extensions built");
    assert!(
        exts.iter().any(|e| e.name() == name),
        "expected native extension '{}', found: {:?}",
        name,
        exts.iter().map(|e| e.name()).collect::<Vec<_>>()
    );
}

#[then(expr = "the native extensions list should not contain {string}")]
fn then_native_exts_not_contain(world: &mut QuectoWorld, name: String) {
    let exts = world
        .native_extensions_built
        .as_ref()
        .expect("no native extensions built");
    assert!(
        !exts.iter().any(|e| e.name() == name),
        "native extension '{}' should not be present",
        name
    );
}

#[then("the web_search native extension should use Brave backend")]
fn then_web_search_uses_brave(world: &mut QuectoWorld) {
    let config = world.config.as_ref().expect("no config");
    assert!(config.tools.web.brave.enabled);
    assert!(!config.tools.web.brave.api_key.is_empty());
    let exts = world
        .native_extensions_built
        .as_ref()
        .expect("no native extensions built");
    assert!(exts.iter().any(|e| e.name() == "web"));
    assert!(
        exts.iter()
            .flat_map(|e| e.0.tools())
            .any(|t| t.definition().name.as_ref() == "web_search")
    );
}

#[then("the web_search native extension should use DuckDuckGo backend")]
fn then_web_search_uses_ddg(world: &mut QuectoWorld) {
    let config = world.config.as_ref().expect("no config");
    assert!(config.tools.web.duckduckgo.enabled);
    assert!(
        config.tools.web.brave.api_key.is_empty() || !config.tools.web.brave.enabled,
        "Brave should not be configured for DDG backend test"
    );
    let exts = world
        .native_extensions_built
        .as_ref()
        .expect("no native extensions built");
    assert!(exts.iter().any(|e| e.name() == "web"));
    assert!(
        exts.iter()
            .flat_map(|e| e.0.tools())
            .any(|t| t.definition().name.as_ref() == "web_search")
    );
}

// ─── Multi-tool native extension steps (#364) ────────────────────────────────

#[given(expr = "a native extension named {string} with tools {string} and {string}")]
fn given_native_ext_multi_tool(
    world: &mut QuectoWorld,
    name: String,
    tool1: String,
    tool2: String,
) {
    let t1: Arc<dyn Tool> = Arc::new(DummyTool {
        name: tool1,
        description: "tool one".to_string(),
    });
    let t2: Arc<dyn Tool> = Arc::new(DummyTool {
        name: tool2,
        description: "tool two".to_string(),
    });
    let ext = NativeExtension::with_tools(name, "multi-tool ext", vec![t1, t2]);
    world.native_extension = Some(DebugExtension(Arc::new(ext)));
}

#[then(expr = "the native extension should provide {int} tools")]
fn then_native_ext_tool_count_multi(world: &mut QuectoWorld, expected: i32) {
    let ext = world
        .native_extension
        .as_ref()
        .expect("no native extension");
    assert_eq!(ext.0.tools().len(), expected as usize);
}

// ─── WebFetchTool config-gating steps (#364) ─────────────────────────────────

#[given("a config with tools.web.fetch.enabled = true")]
fn given_config_fetch_enabled(world: &mut QuectoWorld) {
    world.config = Some(config_with_web(false, "", false, true));
}

#[given("a config with tools.web.fetch.enabled = false")]
fn given_config_fetch_disabled(world: &mut QuectoWorld) {
    world.config = Some(config_with_web(false, "", false, false));
}

#[given(
    expr = "a config with tools.web.brave.enabled = true and api_key = {string} and fetch.enabled = true"
)]
fn given_config_brave_and_fetch(world: &mut QuectoWorld, api_key: String) {
    world.config = Some(config_with_web(true, &api_key, false, true));
}

#[given(
    expr = "a config with tools.web.brave.enabled = true and api_key = {string} and fetch.enabled = false"
)]
fn given_config_brave_no_fetch(world: &mut QuectoWorld, api_key: String) {
    world.config = Some(config_with_web(true, &api_key, false, false));
}

#[given("a config with tools.web.fetch.enabled = true and search disabled")]
fn given_config_fetch_only(world: &mut QuectoWorld) {
    world.config = Some(config_with_web(false, "", false, true));
}

#[given("config with tools.web.fetch.enabled = false")]
fn given_config_fetch_also_disabled(world: &mut QuectoWorld) {
    // Modify existing config to also disable fetch
    let config = world.config.as_mut().expect("no config");
    config.tools.web.fetch.enabled = false;
}

#[then(expr = "the built native extension {string} should provide tool {string}")]
fn then_built_ext_provides_tool(world: &mut QuectoWorld, ext_name: String, tool_name: String) {
    let exts = world
        .native_extensions_built
        .as_ref()
        .expect("no native extensions built");
    let ext = exts
        .iter()
        .find(|e| e.name() == ext_name)
        .unwrap_or_else(|| panic!("extension '{}' not found", ext_name));
    let tools = ext.0.tools();
    assert!(
        tools
            .iter()
            .any(|t| t.definition().name.as_ref() == tool_name),
        "extension '{}' does not provide tool '{}', has: {:?}",
        ext_name,
        tool_name,
        tools
            .iter()
            .map(|t| t.definition().name.to_string())
            .collect::<Vec<_>>()
    );
}

#[then(expr = "there should be no built native extension providing tool {string}")]
fn then_no_ext_provides_tool(world: &mut QuectoWorld, tool_name: String) {
    let exts = world
        .native_extensions_built
        .as_ref()
        .expect("no native extensions built");
    for ext in exts.iter() {
        let tools = ext.0.tools();
        assert!(
            !tools
                .iter()
                .any(|t| t.definition().name.as_ref() == tool_name),
            "extension '{}' unexpectedly provides tool '{}'",
            ext.name(),
            tool_name
        );
    }
}

// ─── Unified tool model and runtime policy (#1276) ─────────────────────────

#[given(expr = "a UDS extension tool {string} is registered")]
#[given(expr = "a UDS runtime capability {string} is registered")]
#[when(expr = "a UDS runtime capability {string} is registered")]
fn given_uds_extension_tool_registered(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let tool = Arc::new(DummyTool {
        name: tool_name,
        description: "UDS extension test tool".to_string(),
    });
    world.tool_policy_change_result = Some(registry.register_uds_tool(tool));
    world.tool_definitions_snapshot = registry.definitions().to_vec();
}

#[given(expr = "UDS client {string} has registered runtime capability {string}")]
fn given_uds_client_registered_capability(
    world: &mut QuectoWorld,
    client_id: String,
    tool_name: String,
) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let tool = Arc::new(DummyTool {
        name: tool_name,
        description: "UDS extension test tool".to_string(),
    });
    let owner = format!("uds:client:{client_id}").into();
    world.tool_policy_change_result = Some(registry.register_uds_tool_for_owner(tool, owner));
    world.tool_definitions_snapshot = registry.definitions().to_vec();
}

#[when(expr = "UDS client {string} registers runtime capability {string}")]
fn when_uds_client_registers_capability(
    world: &mut QuectoWorld,
    client_id: String,
    tool_name: String,
) {
    given_uds_client_registered_capability(world, client_id, tool_name);
}

#[when(expr = "UDS client {string} disconnects")]
fn when_uds_client_disconnects(world: &mut QuectoWorld, client_id: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let owner = format!("uds:client:{client_id}");
    registry.unregister_extensions_for_owner(owner.as_str());
    world.tool_definitions_snapshot = registry.definitions().to_vec();
}

#[when(expr = "tool {string} is disabled at runtime")]
fn when_tool_disabled_at_runtime(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let changed = registry.disable_tool(&tool_name);
    world.tool_policy_change_result = Some(changed);
    assert!(
        changed,
        "expected registered tool '{}' to disable",
        tool_name
    );
    world.tool_definitions_snapshot = registry.definitions().to_vec();
}

#[given(expr = "tool {string} is disabled at runtime")]
fn given_tool_disabled_at_runtime(world: &mut QuectoWorld, tool_name: String) {
    when_tool_disabled_at_runtime(world, tool_name);
}

#[when(expr = "tool {string} is enabled at runtime")]
fn when_tool_enabled_at_runtime(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let changed = registry.enable_tool(&tool_name);
    world.tool_policy_change_result = Some(changed);
    assert!(
        changed,
        "expected registered tool '{}' to enable",
        tool_name
    );
    world.tool_definitions_snapshot = registry.definitions().to_vec();
}

#[then(expr = "the tool descriptor for {string} should have source {string}")]
fn then_tool_descriptor_source(world: &mut QuectoWorld, tool_name: String, source: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.source.as_str(), source);
}

#[then(expr = "the tool descriptor for {string} should have owner {string}")]
fn then_tool_descriptor_owner(world: &mut QuectoWorld, tool_name: String, owner: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.owner.as_ref(), owner);
}

#[then(expr = "the tool descriptor for {string} should be configured disabled")]
fn then_tool_descriptor_disabled(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.availability.as_str(), "disabled");
}

#[then(expr = "the tool descriptor for {string} should be configured enabled")]
fn then_tool_descriptor_enabled(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.availability.as_str(), "enabled");
}

#[then(expr = "the model-callable catalogue should not offer {string}")]
fn then_model_callable_catalogue_excludes(world: &mut QuectoWorld, tool_name: String) {
    assert!(
        !world
            .tool_definitions_snapshot
            .iter()
            .any(|definition| definition.name.as_ref() == tool_name),
        "model-visible definitions unexpectedly contained '{}'",
        tool_name
    );
}

#[then(expr = "the model-callable catalogue should offer {string}")]
fn then_model_callable_catalogue_includes(world: &mut QuectoWorld, tool_name: String) {
    assert!(
        world
            .tool_definitions_snapshot
            .iter()
            .any(|definition| definition.name.as_ref() == tool_name),
        "model-visible definitions did not contain '{}'",
        tool_name
    );
}

#[then(expr = "executing tool {string} should be rejected as disabled")]
fn then_executing_tool_rejected_disabled(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, "{}"))
        .expect("tool execution should return a policy ToolResult");
    assert!(result.is_error, "disabled tool should return is_error");
    assert!(
        result.content.contains("disabled"),
        "disabled tool result should explain policy, got: {}",
        result.content
    );
}

#[then(
    expr = "the capability catalogue should describe {string} as a bundled native capability owned by Quecto"
)]
fn then_catalogue_describes_bundled_native(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.source.as_str(), "bundled-native");
    assert_eq!(descriptor.owner.as_ref(), "quecto:official-tools");
}

#[then(expr = "the capability catalogue should describe {string} as a UDS runtime capability")]
fn then_catalogue_describes_uds_runtime(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.source.as_str(), "uds");
    assert_eq!(descriptor.owner.as_ref(), "uds:runtime");
}

#[then(expr = "the capability catalogue should list {string} as disabled")]
fn then_catalogue_lists_disabled(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.availability.as_str(), "disabled");
}

#[then(expr = "the capability catalogue should list {string} as enabled")]
fn then_catalogue_lists_enabled(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.availability.as_str(), "enabled");
}

#[then(expr = "the capability catalogue should list {string} as owned by UDS client {string}")]
fn then_catalogue_lists_uds_client_owner(
    world: &mut QuectoWorld,
    tool_name: String,
    client_id: String,
) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let descriptor = registry
        .descriptor(&tool_name)
        .unwrap_or_else(|| panic!("missing descriptor for '{}'", tool_name));
    assert_eq!(descriptor.source.as_str(), "uds");
    assert_eq!(descriptor.owner.as_ref(), format!("uds:client:{client_id}"));
}

#[then(expr = "the runtime capability registration should be rejected")]
fn then_runtime_capability_registration_rejected(world: &mut QuectoWorld) {
    assert_eq!(world.tool_policy_change_result, Some(false));
}

#[when(expr = "executing tool {string} is attempted")]
#[then(expr = "executing tool {string} is attempted")]
fn then_executing_tool_attempted(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, "{}"))
        .map_err(|err| err.to_string());
    world.tool_result = Some(result);
}

#[when(expr = "executing tool {string} is attempted with command {string}")]
#[then(expr = "executing tool {string} is attempted with command {string}")]
fn then_executing_tool_attempted_with_command(
    world: &mut QuectoWorld,
    tool_name: String,
    command: String,
) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let arguments = serde_json::json!({ "command": command }).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, &arguments))
        .map_err(|err| err.to_string());
    world.tool_result = Some(result);
}

#[then(expr = "the tool execution should be rejected as disabled")]
fn then_tool_execution_rejected_disabled(world: &mut QuectoWorld) {
    let result = world
        .tool_result
        .as_ref()
        .expect("no tool execution result captured")
        .as_ref()
        .expect("tool execution should return a policy ToolResult");
    assert!(result.is_error, "disabled tool should return is_error");
    assert!(
        result.content.contains("disabled"),
        "disabled tool result should explain policy, got: {}",
        result.content
    );
}

#[then(expr = "the tool execution should succeed with content {string}")]
fn then_tool_execution_succeeds_with_content(world: &mut QuectoWorld, expected: String) {
    let result = world
        .tool_result
        .as_ref()
        .expect("no tool execution result captured")
        .as_ref()
        .expect("tool execution should succeed");
    assert!(
        !result.is_error,
        "tool execution returned error: {}",
        result.content
    );
    assert!(
        result.content.contains(&expected),
        "tool execution content did not contain '{}': {}",
        expected,
        result.content
    );
}

#[when(expr = "unknown tool {string} is disabled at runtime")]
fn when_unknown_tool_disabled_at_runtime(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    world.tool_policy_change_result = Some(registry.disable_tool(&tool_name));
    world.tool_definitions_snapshot = registry.definitions().to_vec();
}

#[then(expr = "the runtime policy change should be rejected")]
fn then_runtime_policy_change_rejected(world: &mut QuectoWorld) {
    assert_eq!(world.tool_policy_change_result, Some(false));
}

#[then(expr = "the tool registry should not contain {string}")]
fn then_tool_registry_not_contains(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let names = registry.names();
    assert!(
        !names.contains(&tool_name),
        "registry should not contain '{}', has: {:?}",
        tool_name,
        names
    );
}
