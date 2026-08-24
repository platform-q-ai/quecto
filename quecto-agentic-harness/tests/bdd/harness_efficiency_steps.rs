//! Step definitions for `harness_efficiency.feature` (issue #996) and
//! `harness_hot_paths.feature` (issue #991).
//!
//! Each scenario is self-contained: state lives on the shared World fields
//! declared for these scenarios, so no new World wiring is required.

use super::*;
use quecto::domain::audit::content_preview;
use quecto::infrastructure::tools::bash::ExecTool;
use quecto::infrastructure::tools::filesystem::{EditTool, ReadTool};
use quecto::infrastructure::tools::web_fetch::strip_html;

#[when("a 500-character multibyte string is previewed to 100 characters")]
fn when_preview_multibyte(world: &mut QuectoWorld) {
    // 500 two-byte 'é' characters — a mid-codepoint cut would corrupt the
    // output or panic when slicing on a non-boundary byte index.
    let input = "é".repeat(500);
    world.efficiency_preview = Some(content_preview(&input, 100));
}

#[then("the preview shows 100 characters ending in an ellipsis")]
fn then_preview_bounded(world: &mut QuectoWorld) {
    let out = world.efficiency_preview.as_ref().expect("preview computed");
    assert_eq!(
        out.chars().count(),
        100,
        "preview must be bounded to 100 chars"
    );
    assert!(
        out.ends_with("..."),
        "truncated preview must end with ellipsis"
    );
}

#[then("every previewed character is a whole codepoint")]
fn then_preview_utf8(world: &mut QuectoWorld) {
    let out = world.efficiency_preview.as_ref().expect("preview computed");
    // The only real UTF-8-safety proof: the kept prefix must be exactly 97
    // intact 'é' characters followed by the "..." ellipsis. A mid-codepoint
    // cut could not reconstruct to whole 'é' chars.
    let expected = format!("{}...", "é".repeat(97));
    assert_eq!(
        *out, expected,
        "preview must cut on a char boundary, not mid-codepoint"
    );
}

#[when("an OpenAI response reports 12 prompt, 7 completion and 19 total tokens")]
fn when_parse_openai_usage(world: &mut QuectoWorld) {
    use quecto::infrastructure::providers::usage;
    let v = serde_json::json!({
        "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19
    });
    world.efficiency_usage = Some(usage::parse_openai_usage(v.as_object().unwrap()));
}

#[then("the recorded usage shows 12 prompt, 7 completion and 12 context tokens")]
fn then_openai_usage(world: &mut QuectoWorld) {
    let u = world.efficiency_usage.as_ref().expect("usage parsed");
    assert_eq!(u.prompt_tokens, 12);
    assert_eq!(u.completion_tokens, 7);
    assert_eq!(u.context_tokens, Some(12));
}

#[when("a Codex response reports 100 input, 40 output and 30 cached tokens")]
fn when_parse_codex_usage(world: &mut QuectoWorld) {
    use quecto::infrastructure::providers::usage;
    let v = serde_json::json!({
        "input_tokens": 100, "output_tokens": 40,
        "input_tokens_details": { "cached_tokens": 30 }
    });
    world.efficiency_usage = Some(usage::parse_codex_usage(v.as_object().unwrap()));
}

#[then("the recorded usage shows 70 prompt, 40 completion and 30 cached tokens")]
fn then_codex_usage(world: &mut QuectoWorld) {
    let u = world.efficiency_usage.as_ref().expect("usage parsed");
    assert_eq!(u.prompt_tokens, 70);
    assert_eq!(u.completion_tokens, 40);
    assert_eq!(u.cache_read_tokens, Some(30));
}

#[when("a provider config written by an older release is loaded")]
fn when_load_legacy_config(world: &mut QuectoWorld) {
    use quecto::infrastructure::config::ProviderEntry;
    // A blob from before the dead `auth_method` field was removed. Serde
    // ignores the now-unknown key, so old on-disk configs still load.
    let json =
        r#"{ "api_key": "sk-x", "api_base": "https://example.test", "auth_method": "api_key" }"#;
    let entry: ProviderEntry = serde_json::from_str(json).expect("legacy config must load");
    world.efficiency_provider_entry = Some(entry);
}

#[then("the config loads and its api_key and api_base are read back")]
fn then_config_loaded(world: &mut QuectoWorld) {
    use quecto::infrastructure::config::ProviderEntry;
    let entry = world
        .efficiency_provider_entry
        .as_ref()
        .expect("config loaded");
    assert_eq!(entry.api_key, "sk-x");
    assert_eq!(entry.api_base, "https://example.test");
    // The absence of any `auth_method` field is enforced at compile time by
    // this exhaustive struct literal — if the field were reintroduced this
    // would fail to build.
    let _explicit = ProviderEntry {
        api_key: "k".into(),
        api_base: "b".into(),
        disable_codex_routing: false,
    };
}

#[given("a large text file is available to the harness")]
fn given_large_text_file(world: &mut QuectoWorld) {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("large.txt");
    let content: String = (1..=20_000).map(|line| format!("line{line}\n")).collect();
    std::fs::write(&path, content).unwrap();
    world.efficiency_workspace = Some(tmp.path().to_path_buf());
    world._efficiency_temp_dir = Some(tmp);
}

#[when("the text file is read from a later line with a small page size")]
fn when_read_later_page(world: &mut QuectoWorld) {
    let workspace = world
        .efficiency_workspace
        .as_ref()
        .expect("workspace prepared")
        .clone();
    let sandbox = Sandbox::new(Some(workspace.clone()));
    let tool = ReadTool::new(Arc::new(workspace), Arc::new(sandbox));
    let result = tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(tool.execute(r#"{"path":"large.txt","offset":19000,"limit":5}"#))
        .expect("read should execute");
    world.efficiency_read_result = Some(result);
}

#[then("only the requested page is shown")]
fn then_requested_page_shown(world: &mut QuectoWorld) {
    let result = world
        .efficiency_read_result
        .as_ref()
        .expect("read result captured");
    assert!(!result.is_error, "read should succeed: {}", result.content);
    assert!(
        result
            .content
            .starts_with("line19000\nline19001\nline19002\nline19003\nline19004")
    );
    assert!(!result.content.contains("line18999"));
    assert!(!result.content.contains("line19005\n"));
}

#[then("the next page guidance is shown")]
fn then_next_page_guidance(world: &mut QuectoWorld) {
    let result = world
        .efficiency_read_result
        .as_ref()
        .expect("read result captured");
    assert!(
        result
            .content
            .contains("996 more lines in file. Use offset=19005 to continue."),
        "expected continuation guidance, got: {}",
        result.content
    );
}

#[given("a workspace for running commands")]
fn given_many_command_lines(world: &mut QuectoWorld) {
    let tmp = TempDir::new().unwrap();
    world.efficiency_workspace = Some(tmp.path().to_path_buf());
    world._efficiency_temp_dir = Some(tmp);
}

#[when("a command producing more log lines than the display limit is executed")]
fn when_command_result_prepared(world: &mut QuectoWorld) {
    let workspace = world
        .efficiency_workspace
        .as_ref()
        .expect("workspace prepared")
        .clone();
    let sandbox = Sandbox::new(Some(workspace.clone()));
    let tool = ExecTool::new(Arc::new(workspace), Arc::new(sandbox));
    let result = tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(tool.execute(r#"{"command":"seq 1 2001 | sed 's/^/log/'"}"#))
        .expect("command should execute");
    assert!(
        !result.is_error,
        "command should succeed: {}",
        result.content
    );
    world.efficiency_command_output = Some(result.content);
}

#[then("the latest log lines are shown")]
fn then_latest_log_lines(world: &mut QuectoWorld) {
    let output = world
        .efficiency_command_output
        .as_ref()
        .expect("command output prepared");
    assert!(output.starts_with("log2\n"), "got: {output}");
    assert!(output.contains("log2001"));
    assert!(!output.contains("log1\n"));
}

#[then("the full output guidance is shown")]
fn then_full_output_guidance(world: &mut QuectoWorld) {
    let output = world
        .efficiency_command_output
        .as_ref()
        .expect("command output prepared");
    assert!(output.contains("Full output ("), "got: {output}");
    assert!(output.contains("saved to:"), "got: {output}");
}

#[given("fetched HTML contains configured non-content regions")]
fn given_html_with_non_content_regions(world: &mut QuectoWorld) {
    world.efficiency_fetched_text = Some(
        "<HEADER>Top</HEADER><main>Article</main><NoScript>Hidden</NoScript><NAV>Menu</NAV>"
            .to_string(),
    );
}

#[when("the fetched HTML is converted to readable text")]
fn when_html_converted(world: &mut QuectoWorld) {
    let html = world.efficiency_fetched_text.take().expect("html prepared");
    world.efficiency_fetched_text = Some(strip_html(&html));
}

#[then("the article text remains visible")]
fn then_article_visible(world: &mut QuectoWorld) {
    let text = world
        .efficiency_fetched_text
        .as_ref()
        .expect("html converted");
    assert_eq!(text, "Article");
}

#[then("the configured non-content regions are hidden")]
fn then_non_content_hidden(world: &mut QuectoWorld) {
    let text = world
        .efficiency_fetched_text
        .as_ref()
        .expect("html converted");
    assert!(!text.contains("Top"));
    assert!(!text.contains("Hidden"));
    assert!(!text.contains("Menu"));
}

#[given("a plain text file without a byte-order mark is available")]
fn given_plain_text_file(world: &mut QuectoWorld) {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("plain.txt"), "first\nsecond\nthird\n").unwrap();
    world.efficiency_workspace = Some(tmp.path().to_path_buf());
    world._efficiency_temp_dir = Some(tmp);
}

#[when("the file is edited with an exact replacement")]
fn when_plain_file_edited(world: &mut QuectoWorld) {
    let workspace = world
        .efficiency_workspace
        .as_ref()
        .expect("workspace prepared")
        .clone();
    let sandbox = Sandbox::new(Some(workspace.clone()));
    let tool = EditTool::new(Arc::new(workspace), Arc::new(sandbox));
    let result = tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(tool.execute(r#"{"path":"plain.txt","oldText":"second","newText":"changed"}"#))
        .expect("edit should execute");
    world.efficiency_edit_result = Some(result);
}

#[then("the file contains the requested replacement")]
fn then_plain_file_changed(world: &mut QuectoWorld) {
    let workspace = world
        .efficiency_workspace
        .as_ref()
        .expect("workspace prepared");
    let content = std::fs::read_to_string(workspace.join("plain.txt")).unwrap();
    assert_eq!(content, "first\nchanged\nthird\n");
}

#[then("the edit confirmation shows the changed line")]
fn then_edit_confirmation_shows_change(world: &mut QuectoWorld) {
    let result = world
        .efficiency_edit_result
        .as_ref()
        .expect("edit result captured");
    assert!(!result.is_error, "edit should succeed: {}", result.content);
    assert!(result.content.contains("Successfully edited plain.txt"));
    assert!(result.content.contains("-2 second"));
    assert!(result.content.contains("+2 changed"));
}
