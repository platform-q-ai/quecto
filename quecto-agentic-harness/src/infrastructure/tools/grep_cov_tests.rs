use super::*;
use tempfile::TempDir;

fn sandbox_for(path: &std::path::Path) -> Sandbox {
    Sandbox::new(Some(path.to_path_buf()), false)
}

#[test]
fn with_rg_binary_sets_definition_and_format_match_block_paths_and_truncation() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("src.txt");
    std::fs::write(&file, "alpha\nbeta gamma\nthird line\n").unwrap();
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(sandbox_for(tmp.path())),
        "custom-rg".into(),
    );
    assert_eq!(tool.rg_cmd(), "custom-rg");
    assert_eq!(tool.definition().name.as_ref(), "grep");

    let mut cache = HashMap::new();
    let mut state = FormatState {
        output_lines: vec![],
        byte_total: 0,
        lines_truncated: false,
        truncated_bytes: false,
    };
    let ws = tmp.path().to_string_lossy().to_string();
    let prefix = format!("{ws}/");
    let cfg = BlockConfig {
        ws_str: &ws,
        ws_prefix_slash: &prefix,
        context_lines: 1,
        max_line_bytes: 4,
        max_output_bytes: 10_000,
    };
    assert!(format_match_block(
        &RgMatch {
            file_path: file,
            line_number: 2
        },
        &mut cache,
        &cfg,
        &mut state
    ));
    assert_eq!(state.output_lines.len(), 3);
    assert!(state.output_lines[1].starts_with("src.txt:2:"));
    assert!(state.lines_truncated);
}

#[test]
fn format_match_block_stops_at_byte_cap() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("f.txt");
    std::fs::write(&file, "a very long line\n").unwrap();
    let mut cache = HashMap::new();
    let mut state = FormatState {
        output_lines: vec![],
        byte_total: 0,
        lines_truncated: false,
        truncated_bytes: false,
    };
    let ws = tmp.path().to_string_lossy().to_string();
    let prefix = format!("{ws}/");
    let cfg = BlockConfig {
        ws_str: &ws,
        ws_prefix_slash: &prefix,
        context_lines: 0,
        max_line_bytes: 100,
        max_output_bytes: 3,
    };
    assert!(!format_match_block(
        &RgMatch {
            file_path: file,
            line_number: 1
        },
        &mut cache,
        &cfg,
        &mut state
    ));
    assert!(state.truncated_bytes);
    assert!(state.output_lines.is_empty());
}

#[tokio::test]
async fn run_rg_reports_missing_binary_as_domain_tool_error() {
    let cmd = tokio::process::Command::new("/definitely/missing/rg-wave3");
    let err = run_rg(cmd).await.unwrap_err().to_string();
    assert!(
        err.contains("rg not found") || err.contains("grep failed to spawn"),
        "{err}"
    );
}
