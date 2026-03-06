use cucumber::{gherkin, given, then, when};
use quecto::domain::error::DomainError;
use quecto::domain::extension::Extension;
use quecto::domain::tool::{Tool, ToolDefinition, ToolResult};
use quecto::infrastructure::extensions::registry::ExtensionRegistry;
use quecto::infrastructure::extensions::script::{
    ExtensionManifest, ScriptTool, discover_script_extensions,
};
use quecto::infrastructure::extensions::watcher::fingerprint_dirs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tempfile::TempDir;

use super::tool_guard_steps::create_test_registry;
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

// ─── Extension manifest steps ────────────────────────────────────────────────

#[given("an extension manifest TOML:")]
fn given_manifest_toml(world: &mut QuectoWorld, step: &gherkin::Step) {
    let docstring = step.docstring().expect("docstring not found").to_string();
    world.ext_manifest_toml = Some(docstring);
}

#[when("I parse the extension manifest")]
fn when_parse_manifest(world: &mut QuectoWorld) {
    let toml = world.ext_manifest_toml.as_ref().expect("no TOML");
    let result = ExtensionManifest::from_toml(toml).map_err(|e| e.to_string());
    world.ext_manifest_result = Some(result);
}

#[when("I try to parse the extension manifest")]
fn when_try_parse_manifest(world: &mut QuectoWorld) {
    let toml = world.ext_manifest_toml.as_ref().expect("no TOML");
    let result = ExtensionManifest::from_toml(toml).map_err(|e| e.to_string());
    world.ext_manifest_result = Some(result);
}

#[then(expr = "the manifest name should be {string}")]
fn then_manifest_name(world: &mut QuectoWorld, expected: String) {
    let manifest = world
        .ext_manifest_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(manifest.name, expected);
}

#[then(expr = "the manifest command should be {string}")]
fn then_manifest_command(world: &mut QuectoWorld, expected: String) {
    let manifest = world
        .ext_manifest_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(manifest.command, expected);
}

#[then(expr = "the manifest timeout should be {int}")]
fn then_manifest_timeout(world: &mut QuectoWorld, expected: i32) {
    let manifest = world
        .ext_manifest_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(manifest.timeout_secs, expected as u64);
}

#[then(expr = "the manifest system prompt should be {string}")]
fn then_manifest_system_prompt(world: &mut QuectoWorld, expected: String) {
    let manifest = world
        .ext_manifest_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(manifest.system_prompt.as_deref(), Some(expected.as_str()));
}

#[then("the manifest system prompt should be None")]
fn then_manifest_system_prompt_none(world: &mut QuectoWorld) {
    let manifest = world
        .ext_manifest_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert!(manifest.system_prompt.is_none());
}

#[then("the manifest parse should fail")]
fn then_manifest_parse_fail(world: &mut QuectoWorld) {
    assert!(
        world.ext_manifest_result.as_ref().unwrap().is_err(),
        "expected parse to fail"
    );
}

// ─── Script tool execution steps ─────────────────────────────────────────────

fn create_script_in_dir(dir: &Path, script_content: &str) -> PathBuf {
    let script_path = dir.join("tool.sh");
    std::fs::write(&script_path, script_content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    script_path
}

fn setup_script_tool(world: &mut QuectoWorld, script_content: &str, timeout_secs: u64) {
    let tmp = TempDir::new().unwrap();
    let script_path = create_script_in_dir(tmp.path(), script_content);
    let manifest = ExtensionManifest {
        name: "test_script".to_string(),
        description: "test".to_string(),
        parameters_schema: r#"{"type":"object"}"#.to_string(),
        command: script_path.to_string_lossy().to_string(),
        timeout_secs,
        system_prompt: None,
    };
    world.ext_script_tool = Some(Arc::new(ScriptTool::new(
        manifest,
        tmp.path().to_path_buf(),
    )));
    world._ext_discover_dir = Some(tmp);
}

#[given(expr = "a script extension with command that outputs {string}")]
fn given_script_outputs(world: &mut QuectoWorld, output: String) {
    let script = format!("#!/bin/sh\necho '{}'\n", output);
    setup_script_tool(world, &script, 30);
}

#[given(expr = "a script extension with command that exits with code {int} and stderr {string}")]
fn given_script_exits_with_error(world: &mut QuectoWorld, code: i32, stderr: String) {
    let script = format!("#!/bin/sh\necho '{}' >&2\nexit {}\n", stderr, code);
    setup_script_tool(world, &script, 30);
}

#[given(expr = "a script extension with command that sleeps for {int} seconds and timeout {int}")]
fn given_script_sleeps(world: &mut QuectoWorld, sleep_secs: i32, timeout: i32) {
    let script = format!("#!/bin/sh\nsleep {}\n", sleep_secs);
    setup_script_tool(world, &script, timeout as u64);
}

#[given(expr = "a script extension with name {string} and description {string}")]
fn given_script_with_name(world: &mut QuectoWorld, name: String, description: String) {
    let tmp = TempDir::new().unwrap();
    let script_path = create_script_in_dir(
        tmp.path(),
        "#!/bin/sh\necho '{\"content\":\"ok\",\"is_error\":false}'",
    );
    let manifest = ExtensionManifest {
        name,
        description,
        parameters_schema: r#"{"type":"object"}"#.to_string(),
        command: script_path.to_string_lossy().to_string(),
        timeout_secs: 30,
        system_prompt: None,
    };
    world.ext_script_tool = Some(Arc::new(ScriptTool::new(
        manifest,
        tmp.path().to_path_buf(),
    )));
    world._ext_discover_dir = Some(tmp);
}

#[when(expr = "I execute the script tool with arguments {string}")]
fn when_execute_script_tool(world: &mut QuectoWorld, arguments: String) {
    let tool = world
        .ext_script_tool
        .as_ref()
        .expect("no script tool")
        .clone();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { tool.execute(&arguments).await });
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the script tool definition name should be {string}")]
fn then_script_tool_name(world: &mut QuectoWorld, expected: String) {
    let tool = world.ext_script_tool.as_ref().expect("no script tool");
    assert_eq!(tool.definition().name.as_ref(), expected);
}

#[then(expr = "the script tool definition description should contain {string}")]
fn then_script_tool_desc_contains(world: &mut QuectoWorld, expected: String) {
    let tool = world.ext_script_tool.as_ref().expect("no script tool");
    assert!(
        tool.definition().description.contains(&expected),
        "expected description to contain '{}'",
        expected
    );
}

// ─── Extension discovery steps ───────────────────────────────────────────────

fn ensure_discover_dir(world: &mut QuectoWorld) -> PathBuf {
    if world._ext_discover_dir.is_none() {
        world._ext_discover_dir = Some(TempDir::new().unwrap());
    }
    world
        ._ext_discover_dir
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf()
}

fn create_extension_in_dir(dir: &Path, name: &str) {
    let ext_dir = dir.join(name);
    std::fs::create_dir_all(&ext_dir).unwrap();
    let manifest = format!(
        r#"name = "{}"
description = "Test extension {}"
parameters_schema = '{{"type":"object"}}'
command = "./tool.sh"
"#,
        name, name
    );
    std::fs::write(ext_dir.join("extension.toml"), manifest).unwrap();
    let script = "#!/bin/sh\necho '{\"content\":\"ok\",\"is_error\":false}'";
    std::fs::write(ext_dir.join("tool.sh"), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            ext_dir.join("tool.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
}

#[given(expr = "a directory with extension {string} containing a valid manifest and script")]
fn given_dir_with_extension(world: &mut QuectoWorld, name: String) {
    let dir = ensure_discover_dir(world);
    create_extension_in_dir(&dir, &name);
}

#[given(expr = "a directory with a subdirectory {string} containing no manifest")]
fn given_dir_with_empty_subdir(world: &mut QuectoWorld, name: String) {
    let dir = ensure_discover_dir(world);
    std::fs::create_dir_all(dir.join(name)).unwrap();
}

#[given(expr = "a directory with extension {string} containing an invalid manifest")]
fn given_dir_with_invalid_manifest(world: &mut QuectoWorld, name: String) {
    let dir = ensure_discover_dir(world);
    let ext_dir = dir.join(name);
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("extension.toml"), "not valid toml {{{{").unwrap();
}

#[when("I discover script extensions from that directory")]
fn when_discover_extensions(world: &mut QuectoWorld) {
    let dir = world
        ._ext_discover_dir
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf();
    let extensions = discover_script_extensions(&dir);
    world.ext_discovered = Some(extensions.into_iter().map(DebugExtension).collect());
}

#[when("I discover script extensions from a non-existent directory")]
fn when_discover_from_nonexistent(world: &mut QuectoWorld) {
    let extensions = discover_script_extensions(Path::new("/nonexistent/path/12345"));
    world.ext_discovered = Some(extensions.into_iter().map(DebugExtension).collect());
}

#[then(expr = "{int} extension(s) should be discovered")]
fn then_extension_count(world: &mut QuectoWorld, expected: i32) {
    let discovered = world.ext_discovered.as_ref().expect("no discovery result");
    assert_eq!(
        discovered.len(),
        expected as usize,
        "expected {} extensions, found {}",
        expected,
        discovered.len()
    );
}

#[then(expr = "the discovered extension should have name {string}")]
fn then_discovered_name(world: &mut QuectoWorld, expected: String) {
    let discovered = world.ext_discovered.as_ref().expect("no discovery result");
    assert!(
        discovered.iter().any(|e| e.0.name() == expected),
        "no extension with name '{}'",
        expected
    );
}

// ─── Hot-reload watcher steps ────────────────────────────────────────────────

fn ensure_watch_dir(world: &mut QuectoWorld) -> PathBuf {
    if world._ext_watch_dir.is_none() {
        world._ext_watch_dir = Some(TempDir::new().unwrap());
    }
    world._ext_watch_dir.as_ref().unwrap().path().to_path_buf()
}

#[given("a watched directory with no extensions")]
fn given_watched_dir_empty(world: &mut QuectoWorld) {
    let _ = ensure_watch_dir(world);
}

#[given(expr = "a watched directory with extension {string}")]
fn given_watched_dir_with_ext(world: &mut QuectoWorld, name: String) {
    let dir = ensure_watch_dir(world);
    create_extension_in_dir(&dir, &name);
}

#[when("I take a fingerprint")]
fn when_take_fingerprint(world: &mut QuectoWorld) {
    let dir = world._ext_watch_dir.as_ref().unwrap().path().to_path_buf();
    let fp = fingerprint_dirs(&[dir]);
    if world.ext_fingerprint_a.is_none() {
        world.ext_fingerprint_a = Some(fp);
    } else {
        world.ext_fingerprint_b = Some(fp);
    }
}

#[when("I take another fingerprint")]
fn when_take_fingerprint_b(world: &mut QuectoWorld) {
    let dir = world._ext_watch_dir.as_ref().unwrap().path().to_path_buf();
    let fp = fingerprint_dirs(&[dir]);
    world.ext_fingerprint_b = Some(fp);
}

#[when(expr = "I add extension {string} to the watched directory")]
fn when_add_ext_to_watched(world: &mut QuectoWorld, name: String) {
    let dir = world._ext_watch_dir.as_ref().unwrap().path().to_path_buf();
    create_extension_in_dir(&dir, &name);
}

#[when(expr = "I remove extension {string} from the watched directory")]
fn when_remove_ext_from_watched(world: &mut QuectoWorld, name: String) {
    let dir = world._ext_watch_dir.as_ref().unwrap().path().to_path_buf();
    std::fs::remove_dir_all(dir.join(name)).unwrap();
}

#[when(expr = "I modify the manifest of extension {string}")]
fn when_modify_manifest(world: &mut QuectoWorld, name: String) {
    let dir = world._ext_watch_dir.as_ref().unwrap().path().to_path_buf();
    let manifest_path = dir.join(&name).join("extension.toml");
    // Ensure mtime changes (some filesystems have 1s granularity)
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let mut content = std::fs::read_to_string(&manifest_path).unwrap();
    content.push_str("\n# modified\n");
    std::fs::write(&manifest_path, content).unwrap();
}

#[then("the fingerprints should differ")]
fn then_fingerprints_differ(world: &mut QuectoWorld) {
    let a = world.ext_fingerprint_a.as_ref().expect("no fingerprint A");
    let b = world.ext_fingerprint_b.as_ref().expect("no fingerprint B");
    assert_ne!(a, b, "expected fingerprints to differ");
}

#[then("the fingerprints should be equal")]
fn then_fingerprints_equal(world: &mut QuectoWorld) {
    let a = world.ext_fingerprint_a.as_ref().expect("no fingerprint A");
    let b = world.ext_fingerprint_b.as_ref().expect("no fingerprint B");
    assert_eq!(a, b, "expected fingerprints to be equal");
}

// ─── Reload steps ────────────────────────────────────────────────────────────

#[given("an extension registry with watched directory")]
fn given_registry_with_watched_dir(world: &mut QuectoWorld) {
    let dir = ensure_watch_dir(world);
    let mut reg = ExtensionRegistry::new();
    reg.set_watch_dirs(vec![dir]);
    world.ext_registry = Some(reg);
}

#[given(expr = "the directory initially has extension {string}")]
fn given_dir_initially_has(world: &mut QuectoWorld, name: String) {
    let dir = world._ext_watch_dir.as_ref().unwrap().path().to_path_buf();
    create_extension_in_dir(&dir, &name);
    // Initial load
    let reg = world.ext_registry.as_mut().expect("need registry");
    reg.reload_scripts();
}

#[given("a tool registry with core tools and an extension registry with watched directory")]
fn given_core_and_ext_registry(world: &mut QuectoWorld) {
    create_test_registry(world);
    let dir = ensure_watch_dir(world);
    let mut reg = ExtensionRegistry::new();
    reg.set_watch_dirs(vec![dir]);
    world.ext_registry = Some(reg);
}

#[when("I reload script extensions")]
fn when_reload_scripts(world: &mut QuectoWorld) {
    let reg = world.ext_registry.as_mut().expect("need registry");
    reg.reload_scripts();
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

#[then(expr = "the registry should not contain tool {string}")]
fn then_registry_not_contains_tool(world: &mut QuectoWorld, name: String) {
    let reg = world.ext_registry.as_ref().expect("need registry");
    let tools = reg.all_tools();
    assert!(
        !tools.iter().any(|t| t.definition().name.as_ref() == name),
        "tool '{}' should not be in registry",
        name
    );
}

#[then(expr = "the core tool {string} should still be in the registry")]
fn then_core_tool_present(world: &mut QuectoWorld, name: String) {
    let reg = world.tool_registry.as_ref().expect("need tool registry");
    assert!(
        reg.get(&name).is_some(),
        "core tool '{}' should still be in registry",
        name
    );
}

// ─── Security hardening steps (#287-#291) ────────────────────────────────────

#[then("creating a script tool should reject the command path")]
fn then_script_tool_rejects_command(world: &mut QuectoWorld) {
    let manifest = world
        .ext_manifest_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    let tmp = TempDir::new().unwrap();
    let result = ScriptTool::try_new(manifest.clone(), tmp.path().to_path_buf());
    assert!(
        result.is_err(),
        "expected command path to be rejected, got Ok"
    );
}

#[given("a script extension with command that outputs 2MiB of data")]
fn given_script_outputs_2mib(world: &mut QuectoWorld) {
    // Use dd to output 2MiB of 'A' characters quickly
    let script = "#!/bin/sh\ndd if=/dev/zero bs=1048576 count=2 2>/dev/null | tr '\\0' 'A'\n";
    setup_script_tool(world, script, 10);
}

#[given(expr = "a directory with a real extension {string}")]
fn given_dir_with_real_ext(world: &mut QuectoWorld, name: String) {
    let dir = ensure_discover_dir(world);
    create_extension_in_dir(&dir, &name);
}

#[given(expr = "a symlink {string} pointing outside the directory")]
fn given_symlink_outside(world: &mut QuectoWorld, name: String) {
    let dir = world
        ._ext_discover_dir
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf();
    let link_path = dir.join(&name);
    // Point to /tmp which is outside the extensions dir
    #[cfg(unix)]
    std::os::unix::fs::symlink("/tmp", &link_path).unwrap();
    #[cfg(not(unix))]
    {
        // On non-unix, just create a regular dir (test will pass vacuously)
        std::fs::create_dir_all(&link_path).unwrap();
    }
}
