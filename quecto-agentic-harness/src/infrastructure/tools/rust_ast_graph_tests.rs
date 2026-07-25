use super::*;
use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;

#[cfg(unix)]
fn symlink_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::os::unix::fs::symlink(src, dst).unwrap();
}

fn tool_with_workspace() -> (RustAstGraphTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("src/nested")).unwrap();
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/lib.rs"),
        r#"
pub mod nested;
use crate::nested::Worker;

pub struct App;
trait Run { fn run(&self); }
impl Run for App { fn run(&self) { helper(); } }
pub async fn start() { helper(); }
fn helper() {}
fn mentioned_in_code() {}
// fn mentioned_in_comment() {}
const S: &str = "mentioned_in_string()";
pub fn unsafe_holder() { unsafe
{ std::ptr::read(1 as *const i32); } }
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/nested/mod.rs"),
        "pub struct Worker;\nimpl Worker { pub fn new() -> Self { Worker } }\n",
    )
    .unwrap();
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
    let tool = RustAstGraphTool::new(Arc::new(tmp.path().to_path_buf()), sandbox);
    (tool, tmp)
}

async fn call(tool: &RustAstGraphTool, args: &str) -> Value {
    let result = tool.execute(args).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    serde_json::from_str(&result.content).unwrap()
}

#[test]
fn definition_is_valid() {
    let (tool, _tmp) = tool_with_workspace();
    let def = tool.definition();
    assert_eq!(def.name, "rust_ast_graph");
    assert!(def.description.contains("Example"));
    serde_json::from_str::<Value>(&def.parameters_schema).unwrap();
}

#[tokio::test]
async fn invalid_json_returns_tool_error() {
    let (tool, _tmp) = tool_with_workspace();
    let result = tool.execute("not json").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid JSON"));
}

#[tokio::test]
async fn overview_lists_modules_and_declarations() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(&tool, r#"{"action":"overview","limit":20}"#).await;
    assert_eq!(v["crates"][0], "demo");
    assert!(v["files"].as_u64().unwrap() >= 2);
    let text = v.to_string();
    assert!(text.contains("App"));
    assert!(text.contains("nested"));
}

#[tokio::test]
async fn find_symbol_reports_ambiguity() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/other.rs"), "pub struct App;\n").unwrap();
    let v = call(&tool, r#"{"action":"find_symbol","symbol":"App"}"#).await;
    assert_eq!(v["ambiguous"], true);
    assert!(v["matches"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn find_symbol_qualified_path_matches_segments_not_suffixes() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(
        tmp.path().join("src/data.rs"),
        "pub struct Data;\npub struct MetaData;\n",
    )
    .unwrap();

    let v = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"Data","limit":10}"#,
    )
    .await;
    assert_eq!(v["ambiguous"], false);
    let matches = v["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["name"], "Data");
}

#[tokio::test]
async fn public_item_after_blank_line_reports_item_line_and_signature() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(
        tmp.path().join("src/blank.rs"),
        "\n\npub fn after_blank() {}\n",
    )
    .unwrap();

    let v = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"after_blank","limit":10}"#,
    )
    .await;
    let symbol = &v["matches"].as_array().unwrap()[0];
    assert_eq!(symbol["location"]["line"], 3);
    assert_eq!(symbol["signature"], "pub fn after_blank() {}");
    assert_eq!(symbol["visibility"], "pub");
}

#[tokio::test]
async fn references_ignore_comments_and_strings_by_default() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(
        &tool,
        r#"{"action":"references","symbol":"mentioned_in_comment","limit":20}"#,
    )
    .await;
    assert_eq!(v["results"].as_array().unwrap().len(), 0);
    let raw = call(
        &tool,
        r#"{"action":"references","symbol":"mentioned_in_comment","raw_text":true,"limit":20}"#,
    )
    .await;
    assert!(!raw["results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn bounded_output_and_query_work() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(
        &tool,
        r#"{"action":"query","query":"async_functions","limit":1,"snippet_lines":1}"#,
    )
    .await;
    assert_eq!(v["results"].as_array().unwrap().len(), 1);
    assert!(v.to_string().contains("start"));
    let unsafe_v = call(&tool, r#"{"action":"query","query":"unsafe_blocks"}"#).await;
    assert!(unsafe_v.to_string().contains("unsafe block"));
}

#[tokio::test]
async fn depth_and_include_bodies_controls_are_exercised() {
    let (tool, _tmp) = tool_with_workspace();
    let shallow = call(
        &tool,
        r#"{"action":"neighbors","symbol":"App","depth":0,"limit":20}"#,
    )
    .await;
    assert_eq!(shallow["depth"], 0);
    assert!(
        shallow["syntactic_call_sites"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let with_bodies = call(
        &tool,
        r#"{"action":"query","query":"functions","include_bodies":true,"limit":5}"#,
    )
    .await;
    assert!(with_bodies.to_string().contains("std::ptr::read"));

    let def = tool.definition();
    assert!(def.parameters_schema.contains("depth"));
    assert!(def.parameters_schema.contains("include_bodies"));
}

#[tokio::test]
async fn neighbors_include_impls_imports_and_calls() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(&tool, r#"{"action":"neighbors","symbol":"App","limit":20}"#).await;
    let text = v.to_string();
    assert!(text.contains("Run"));
    assert!(text.contains("helper"));
    assert!(text.contains("nested::Worker"));
}

#[tokio::test]
async fn sandbox_blocks_outside_path() {
    let (tool, _tmp) = tool_with_workspace();
    let result = tool
        .execute(r#"{"action":"overview","path":"/"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("outside working dir"));
}

#[tokio::test]
async fn calls_action_finds_syntactic_call_candidates() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(&tool, r#"{"action":"calls","symbol":"helper","limit":10}"#).await;
    assert!(v["calls_only"].as_bool().unwrap());
    assert!(v.to_string().contains("syntactic call candidate"));
}

#[tokio::test]
async fn malformed_rust_returns_partial_diagnostic() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/broken.rs"), "pub fn broken( {\n").unwrap();

    let v = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"broken","limit":10}"#,
    )
    .await;
    assert!(v["matches"].as_array().unwrap().len() <= 1);
    assert!(v.to_string().contains("partial parse diagnostic"));
    assert!(v.to_string().contains("broken.rs"));
}

#[tokio::test]
async fn missing_fields_and_unknown_action_are_tool_errors() {
    let (tool, _tmp) = tool_with_workspace();
    let missing = tool.execute(r#"{"action":"find_symbol"}"#).await.unwrap();
    assert!(missing.is_error);
    assert!(missing.content.contains("missing required field"));

    let unknown = tool.execute(r#"{"action":"wat"}"#).await.unwrap();
    assert!(unknown.is_error);
    assert!(unknown.content.contains("unsupported action"));
}

#[tokio::test]
async fn scoped_file_and_missing_directory_paths_are_handled() {
    let (tool, _tmp) = tool_with_workspace();
    let file_scope = call(
        &tool,
        r#"{"action":"overview","path":"src/lib.rs","limit":50}"#,
    )
    .await;
    assert_eq!(file_scope["files"], 1);

    let missing = tool
        .execute(r#"{"action":"overview","path":"src/does-not-exist"}"#)
        .await
        .unwrap();
    assert!(missing.is_error);
    assert!(missing.content.contains("failed to read"));
}

#[tokio::test]
async fn query_variants_and_ambiguous_neighbor_error_are_covered() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/other.rs"), "pub struct App;\n").unwrap();

    for query in ["trait_impls", "public_api", "functions"] {
        let v = call(
            &tool,
            &format!(r#"{{"action":"query","query":"{query}","limit":5}}"#),
        )
        .await;
        assert_eq!(v["query"], query);
    }

    let unknown = tool
        .execute(r#"{"action":"query","query":"unknown_query"}"#)
        .await
        .unwrap();
    assert!(unknown.is_error);
    assert!(unknown.content.contains("unsupported query"));

    let ambiguous = tool
        .execute(r#"{"action":"neighbors","symbol":"App"}"#)
        .await
        .unwrap();
    assert!(ambiguous.is_error);
    assert!(ambiguous.content.contains("ambiguous symbol"));
}

#[tokio::test]
async fn references_to_selected_symbol_cover_resolved_path() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(
        &tool,
        r#"{"action":"references","symbol":"helper","limit":10}"#,
    )
    .await;
    assert_eq!(v["target"], "helper");
    assert!(!v["results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn masking_handles_raw_strings_lifetimes_and_non_ascii_offsets() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(
        tmp.path().join("src/unicode.rs"),
        "pub fn café() {}\nconst RAW: &str = r#\"fn fake_raw() {}\"#;\nfn lifetime_arg(x: &'static str) {}\nfn after_lifetime() {}\n",
    )
    .unwrap();

    let fake = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"fake_raw","limit":10}"#,
    )
    .await;
    assert!(fake["matches"].as_array().unwrap().is_empty());

    let after = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"after_lifetime","limit":10}"#,
    )
    .await;
    assert_eq!(after["matches"].as_array().unwrap().len(), 1);

    let accented = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"lifetime_arg","limit":10}"#,
    )
    .await;
    assert!(accented.to_string().contains("lifetime_arg"));
}

#[tokio::test]
async fn symlinked_rs_files_outside_sandbox_are_skipped() {
    let (tool, tmp) = tool_with_workspace();
    let outside = TempDir::new().unwrap();
    let outside_file = outside.path().join("secret.rs");
    std::fs::write(&outside_file, "pub fn leaked_secret() {}\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_file, tmp.path().join("src/leak.rs")).unwrap();

    let v = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"leaked_secret","limit":10}"#,
    )
    .await;
    assert!(v["matches"].as_array().unwrap().is_empty());
    assert!(v.to_string().contains("outside sandbox"));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_directory_cycles_are_skipped() {
    let (tool, tmp) = tool_with_workspace();
    symlink_dir(tmp.path(), &tmp.path().join("src/loop"));

    let v = call(&tool, r#"{"action":"overview","limit":10}"#).await;
    assert!(v.to_string().contains("already-visited directory"));
}

#[tokio::test]
async fn workspace_member_module_paths_are_crate_relative() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::create_dir_all(tmp.path().join("crates/foo/src/nested")).unwrap();
    std::fs::write(
        tmp.path().join("crates/foo/Cargo.toml"),
        "[package]\nname=\"foo\"\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("crates/foo/src/nested/mod.rs"),
        "pub struct MemberThing;\n",
    )
    .unwrap();
    let v = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"MemberThing","limit":10}"#,
    )
    .await;
    assert!(v.to_string().contains("nested::MemberThing"));
    assert!(!v.to_string().contains("crates::foo::src"));
}
