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
const C: char = 'a';
pub fn after_char_literal() {}
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
    let schema: Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["action"]["type"], "string");
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    for action in [
        "overview",
        "find_symbol",
        "neighbors",
        "references",
        "calls",
        "query",
    ] {
        assert!(actions.iter().any(|value| value == action));
    }
    assert_eq!(schema["properties"]["limit"]["minimum"], 1);
    assert_eq!(schema["properties"]["limit"]["maximum"], 200);
    assert_eq!(schema["properties"]["depth"]["maximum"], 5);
    assert_eq!(schema["properties"]["snippet_lines"]["maximum"], 20);
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

    let without_bodies = call(
        &tool,
        r#"{"action":"query","query":"functions","limit":20}"#,
    )
    .await;
    let without_body_results = without_bodies["results"].as_array().unwrap();
    assert!(
        without_body_results
            .iter()
            .any(|result| result["name"] == "unsafe_holder")
    );
    assert!(
        without_body_results
            .iter()
            .all(|result| result["snippet"].as_str().unwrap().is_empty())
    );

    let with_bodies = call(
        &tool,
        r#"{"action":"query","query":"functions","include_bodies":true,"limit":20}"#,
    )
    .await;
    let unsafe_holder = with_bodies["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["name"] == "unsafe_holder")
        .unwrap();
    assert!(
        unsafe_holder["snippet"]
            .as_str()
            .unwrap()
            .contains("std::ptr::read")
    );

    let no_snippet = call(
        &tool,
        r#"{"action":"query","query":"unsafe_blocks","snippet_lines":0,"limit":1}"#,
    )
    .await;
    let snippet = no_snippet["results"][0]["snippet"].as_str().unwrap();
    assert!(snippet.is_empty());
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
    assert!(!missing.content.contains("partial parse diagnostic"));

    let missing_query = tool.execute(r#"{"action":"query"}"#).await.unwrap();
    assert!(missing_query.is_error);
    assert!(
        missing_query
            .content
            .contains("missing required field 'query'")
    );

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

    let trait_impls = call(
        &tool,
        r#"{"action":"query","query":"trait_impls","limit":5}"#,
    )
    .await;
    let impl_result = &trait_impls["results"].as_array().unwrap()[0];
    assert_eq!(impl_result["kind"], "impl");
    assert_eq!(impl_result["trait_name"], "Run");
    assert_eq!(impl_result["for_type"], "App");

    let public_api = call(
        &tool,
        r#"{"action":"query","query":"public_api","limit":20}"#,
    )
    .await;
    let public_names: Vec<&str> = public_api["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["name"].as_str().unwrap())
        .collect();
    assert!(public_names.contains(&"App"));
    assert!(public_names.contains(&"start"));
    assert!(public_names.contains(&"Worker"));

    let functions = call(
        &tool,
        r#"{"action":"query","query":"functions","limit":20}"#,
    )
    .await;
    let function_results = functions["results"].as_array().unwrap();
    assert!(function_results.iter().all(|value| value["kind"] == "fn"));
    let function_names: Vec<&str> = function_results
        .iter()
        .map(|value| value["name"].as_str().unwrap())
        .collect();
    assert!(function_names.contains(&"helper"));
    assert!(function_names.contains(&"start"));

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
    let found = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"helper","limit":10}"#,
    )
    .await;
    let helper_id = found["matches"][0]["id"].as_str().unwrap();
    let v = call(
        &tool,
        &format!(r#"{{"action":"references","symbol":"{helper_id}","limit":10}}"#),
    )
    .await;
    assert_eq!(v["target"], "helper");
    let lines: Vec<u64> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["line"].as_u64().unwrap())
        .collect();
    assert!(lines.contains(&7), "expected impl call site in {lines:?}");
    assert!(
        lines.contains(&8),
        "expected async fn call site in {lines:?}"
    );
    assert!(lines.contains(&9), "expected declaration hit in {lines:?}");

    let call_candidates = call(&tool, r#"{"action":"calls","symbol":"helper","limit":10}"#).await;
    let call_lines: Vec<u64> = call_candidates["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["line"].as_u64().unwrap())
        .collect();
    assert_eq!(call_lines, vec![7, 8]);
    assert!(
        call_candidates["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value["snippet"].as_str().unwrap().contains("helper();"))
    );
}

#[tokio::test]
async fn masking_handles_raw_strings_lifetimes_and_non_ascii_offsets() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(
        tmp.path().join("src/unicode.rs"),
        "pub fn café() {}\nconst RAW: &str = r#\"fn fake_raw() {}\"#;\nfn lifetime_arg(x: &'static str) {}\nfn after_lifetime() {}\nconst CHAR_LITERAL: char = 'a';\npub fn after_char_literal_in_extra_file() {}\n",
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

    let after_char = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"after_char_literal_in_extra_file","limit":10}"#,
    )
    .await;
    assert_eq!(after_char["matches"].as_array().unwrap().len(), 1);

    let accented = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"café","limit":10}"#,
    )
    .await;
    let cafe = &accented["matches"].as_array().unwrap()[0];
    assert_eq!(cafe["name"], "café");
    assert_eq!(cafe["signature"], "pub fn café() {}");
    assert_eq!(cafe["location"]["line"], 1);
    assert_eq!(cafe["location"]["column"], 1);
    assert_eq!(cafe["location"]["byte_end"], 12);
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
async fn symlinked_directories_outside_sandbox_are_skipped() {
    let (tool, tmp) = tool_with_workspace();
    let outside = TempDir::new().unwrap();
    std::fs::write(
        outside.path().join("secret.rs"),
        "pub fn leaked_dir_secret() {}
",
    )
    .unwrap();
    symlink_dir(outside.path(), &tmp.path().join("src/external"));

    let v = call(&tool, r#"{"action":"overview","limit":10}"#).await;
    assert!(v.to_string().contains("skipped directory"));
    assert!(!v.to_string().contains("leaked_dir_secret"));
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
async fn numeric_context_controls_are_clamped_at_documented_bounds() {
    let (tool, tmp) = tool_with_workspace();
    let generated = (0..205)
        .map(|idx| format!("fn generated_{idx}() {{}}\n"))
        .collect::<String>();
    std::fs::write(tmp.path().join("src/generated.rs"), generated).unwrap();

    let limit_zero = call(&tool, r#"{"action":"query","query":"functions","limit":0}"#).await;
    assert_eq!(limit_zero["results"].as_array().unwrap().len(), 1);

    let limit_one = call(&tool, r#"{"action":"query","query":"functions","limit":1}"#).await;
    assert_eq!(limit_one["results"].as_array().unwrap().len(), 1);

    let limit_max = call(
        &tool,
        r#"{"action":"query","query":"functions","limit":200}"#,
    )
    .await;
    assert_eq!(limit_max["results"].as_array().unwrap().len(), 200);

    let limit_above_max = call(
        &tool,
        r#"{"action":"query","query":"functions","limit":201}"#,
    )
    .await;
    assert_eq!(limit_above_max["results"].as_array().unwrap().len(), 200);

    let depth_zero = call(
        &tool,
        r#"{"action":"neighbors","symbol":"helper","depth":0,"limit":20}"#,
    )
    .await;
    assert_eq!(depth_zero["depth"], 0);
    assert!(
        depth_zero["syntactic_call_sites"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let depth_max = call(
        &tool,
        r#"{"action":"neighbors","symbol":"helper","depth":5,"limit":20}"#,
    )
    .await;
    let depth_above_max = call(
        &tool,
        r#"{"action":"neighbors","symbol":"helper","depth":6,"limit":20}"#,
    )
    .await;
    assert_eq!(depth_max["depth"], 5);
    assert_eq!(depth_above_max["depth"], 5);
    assert_eq!(
        depth_max["syntactic_call_sites"],
        depth_above_max["syntactic_call_sites"]
    );

    let snippet_zero = call(
        &tool,
        r#"{"action":"query","query":"functions","snippet_lines":0,"limit":1}"#,
    )
    .await;
    assert_eq!(snippet_zero["results"][0]["snippet"], "");

    let snippet_max = call(
        &tool,
        r#"{"action":"query","query":"functions","snippet_lines":20,"limit":1,"include_bodies":true}"#,
    )
    .await;
    let snippet_above_max = call(
        &tool,
        r#"{"action":"query","query":"functions","snippet_lines":21,"limit":1,"include_bodies":true}"#,
    )
    .await;
    assert_eq!(
        snippet_max["results"][0]["snippet"],
        snippet_above_max["results"][0]["snippet"]
    );
}

#[tokio::test]
async fn file_collection_stops_at_global_rust_file_limit() {
    let (tool, tmp) = tool_with_workspace();
    let first = tmp.path().join("src/limit_a");
    let second = tmp.path().join("src/limit_b");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    for idx in 0..crate::infrastructure::tools::rust_ast_graph_parse::MAX_RUST_FILES {
        std::fs::write(
            first.join(format!("generated_{idx}.rs")),
            "fn generated() {}\n",
        )
        .unwrap();
    }
    std::fs::write(
        second.join("should_not_overflow.rs"),
        "fn should_not_overflow() {}\n",
    )
    .unwrap();

    let overview = call(&tool, r#"{"action":"overview","path":"src","limit":1}"#).await;
    assert_eq!(
        overview["files"].as_u64().unwrap(),
        crate::infrastructure::tools::rust_ast_graph_parse::MAX_RUST_FILES as u64
    );
    assert!(
        overview
            .to_string()
            .contains("stopped after MAX_RUST_FILES=2000")
    );
}

#[tokio::test]
async fn file_size_limit_keeps_exact_limit_file_and_skips_oversized_file() {
    let (tool, tmp) = tool_with_workspace();
    let at_limit_decl = "pub fn at_file_size_limit() {}\n";
    let at_limit_padding = crate::infrastructure::tools::rust_ast_graph_parse::MAX_FILE_BYTES
        as usize
        - at_limit_decl.len()
        - "//".len();
    std::fs::write(
        tmp.path().join("src/at_limit.rs"),
        format!("{at_limit_decl}//{}", "x".repeat(at_limit_padding)),
    )
    .unwrap();

    let oversized_decl = "pub fn over_file_size_limit() {}\n";
    let oversized_padding =
        crate::infrastructure::tools::rust_ast_graph_parse::MAX_FILE_BYTES as usize + 1
            - oversized_decl.len()
            - "//".len();
    std::fs::write(
        tmp.path().join("src/over_limit.rs"),
        format!("{oversized_decl}//{}", "x".repeat(oversized_padding)),
    )
    .unwrap();

    let included = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"at_file_size_limit","limit":10}"#,
    )
    .await;
    assert_eq!(included["matches"].as_array().unwrap().len(), 1);

    let skipped = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"over_file_size_limit","limit":10}"#,
    )
    .await;
    assert!(skipped["matches"].as_array().unwrap().is_empty());
    assert!(skipped.to_string().contains("over_limit.rs"));
    assert!(
        skipped
            .to_string()
            .contains("skipped by rust_ast_graph size limit")
    );
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
