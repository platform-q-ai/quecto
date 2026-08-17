use super::*;
use crate::infrastructure::security::sandbox::Sandbox;
use tempfile::TempDir;

fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), false));
    (workspace, sandbox, tmp)
}

#[tokio::test]
async fn test_read_file_outside_workspace_allowed() {
    let (ws, sb, _tmp) = test_tools();
    let outside = TempDir::new().unwrap();
    let file = outside.path().join("outside.txt");
    std::fs::write(&file, "outside").unwrap();
    let tool = ReadTool::new(ws, sb);
    let result = tool
        .execute(&format!(r#"{{"path": "{}"}}"#, file.display()))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("outside"));
}

#[tokio::test]
async fn test_read_truncates_large_file() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);

    let content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
    std::fs::write(tmp.path().join("big.txt"), &content).unwrap();

    let result = tool.execute(r#"{"path": "big.txt"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("Showing lines"),
        "expected truncation hint"
    );
}

#[tokio::test]
async fn test_read_offset_pagination() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let content: String = (1..=10).map(|i| format!("line{}\n", i)).collect();
    std::fs::write(tmp.path().join("paged.txt"), &content).unwrap();

    let result = tool
        .execute(r#"{"path": "paged.txt", "offset": 5, "limit": 3}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("line5"));
    assert!(result.content.contains("line7"));
    assert!(!result.content.contains("line4"));
    assert!(!result.content.contains("line8"));
}

/// #1316: models often emit integral floats (`370.0`) for schema `"number"`.
/// Those must honor the requested window — never silently fall back to default head.
#[tokio::test]
async fn test_read_float_offset_limit_does_not_return_default_head() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);

    let content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
    std::fs::write(tmp.path().join("big.txt"), &content).unwrap();

    let result = tool
        .execute(r#"{"path": "big.txt", "offset": 100.0, "limit": 5.0}"#)
        .await
        .expect("integral float offset/limit must succeed");
    assert!(
        !result.is_error,
        "unexpected tool error: {}",
        result.content
    );
    let first_lines: Vec<&str> = result.content.lines().take(5).collect();
    assert_eq!(
        first_lines,
        ["line100", "line101", "line102", "line103", "line104"],
        "expected exact 5-line window starting at line100"
    );
    assert!(
        !result.content.lines().any(|l| l == "line1"),
        "must not return default head from line1; got prefix: {}",
        first_lines.join("\n")
    );
    assert!(!result.content.lines().any(|l| l == "line99"));
    assert!(!result.content.lines().any(|l| l == "line105"));
}

#[tokio::test]
async fn test_read_fractional_offset_is_error() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("f.txt"), "a\nb\nc\nd\ne\n").unwrap();

    let result = tool
        .execute(r#"{"path": "f.txt", "offset": 3.5, "limit": 10}"#)
        .await;
    assert!(
        result.is_err(),
        "fractional offset must error, not default-head"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("offset"),
        "error should name the field, got: {msg}"
    );
}

#[tokio::test]
async fn test_read_negative_limit_is_error() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("f.txt"), "a\nb\nc\n").unwrap();

    let result = tool
        .execute(r#"{"path": "f.txt", "offset": 1, "limit": -1}"#)
        .await;
    assert!(result.is_err(), "negative limit must error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("limit"),
        "error should name the field, got: {msg}"
    );
}

#[tokio::test]
async fn test_read_string_offset_is_error() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("f.txt"), "a\nb\nc\n").unwrap();

    let result = tool
        .execute(r#"{"path": "f.txt", "offset": "5", "limit": 1}"#)
        .await;
    assert!(
        result.is_err(),
        "string offset must error, no string coerce"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("offset"),
        "error should name the field, got: {msg}"
    );
}

/// Direct parser boundary: values above `usize::MAX` must Err, never truncate via `as usize`.
/// On 64-bit, construct a JSON integer u64 that cannot fit in usize only when usize < u64;
/// always exercise a value that fails the fit check via `serde_json::Number`.
#[test]
fn test_parse_optional_usize_arg_rejects_above_usize_max() {
    // Build a Number from u64::MAX. On 32-bit platforms usize::try_from fails.
    // On 64-bit, u64::MAX == usize::MAX so try_from succeeds — also test a float
    // path value larger than usize::MAX when representable, and the integer path
    // with an explicit value known to exceed via cfg.
    #[cfg(target_pointer_width = "32")]
    {
        let v = serde_json::Value::Number(serde_json::Number::from(u64::MAX));
        let err = parse_optional_usize_arg(&v, "offset").unwrap_err();
        assert!(
            err.contains("offset") && err.contains("out of range"),
            "expected out-of-range error, got: {err}"
        );
    }

    // Exact 2^64 boundary: f64 can represent 18446744073709551616.0 exactly.
    // On 64-bit, `usize::MAX as f64` rounds UP to this same value, so a naive
    // `f > (usize::MAX as f64)` bound incorrectly accepts 2^64 and saturates
    // via `f as u64` → u64::MAX. Must Err, never Ok(Some(usize::MAX)).
    {
        const TWO_POW_64: f64 = 18446744073709551616.0; // 2^64, exact in f64
        assert_eq!(TWO_POW_64, (u64::MAX as f64) + 1.0); // sanity: exact power of two
        let n = serde_json::Number::from_f64(TWO_POW_64).expect("2^64 is a finite JSON number");
        let v = serde_json::Value::Number(n);
        let err = parse_optional_usize_arg(&v, "limit").expect_err(
            "2^64 float must be rejected (outside usize); must not saturate to usize::MAX",
        );
        assert!(
            err.contains("limit") && err.contains("out of range"),
            "expected out-of-range error for 2^64, got: {err}"
        );
    }

    #[cfg(target_pointer_width = "64")]
    {
        // Still probe a value well above the bound (2 * rounded usize::MAX).
        let above = (usize::MAX as f64) * 2.0;
        assert!(above.is_finite());
        let n = serde_json::Number::from_f64(above).expect("finite f64 should be a JSON number");
        let v = serde_json::Value::Number(n);
        let err = parse_optional_usize_arg(&v, "limit").unwrap_err();
        assert!(
            err.contains("limit") && err.contains("out of range"),
            "expected out-of-range error, got: {err}"
        );
    }

    // Sanity: in-range zero and integral float still parse (0 allowed at parser layer).
    assert_eq!(
        parse_optional_usize_arg(&serde_json::json!(0), "offset").unwrap(),
        Some(0)
    );
    assert_eq!(
        parse_optional_usize_arg(&serde_json::json!(50.0), "limit").unwrap(),
        Some(50)
    );
    // Negative integer path (i64), negative float, wrong type, and -0.0.
    let err = parse_optional_usize_arg(&serde_json::json!(-3), "offset").unwrap_err();
    assert!(
        err.contains("offset") && err.contains("non-negative"),
        "{err}"
    );
    let err = parse_optional_usize_arg(&serde_json::json!(-1.5), "limit").unwrap_err();
    assert!(
        err.contains("limit") && err.contains("non-negative"),
        "{err}"
    );
    let err = parse_optional_usize_arg(&serde_json::json!(true), "offset").unwrap_err();
    assert!(
        err.contains("offset") && err.contains("expected a number"),
        "{err}"
    );
    assert_eq!(
        parse_optional_usize_arg(&serde_json::json!(-0.0), "offset").unwrap(),
        Some(0)
    );
    let err = parse_optional_usize_arg(&serde_json::json!(3.5), "limit").unwrap_err();
    assert!(err.contains("limit") && err.contains("integer"), "{err}");
}

#[tokio::test]
async fn test_read_null_offset_and_limit_use_defaults() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("f.txt"), "hello\nworld\n").unwrap();

    let result = tool
        .execute(r#"{"path": "f.txt", "offset": null, "limit": null}"#)
        .await
        .expect("null paging args are treated as absent");
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("hello"));
    assert!(result.content.contains("world"));
}

#[tokio::test]
async fn test_read_offset_beyond_eof_error() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("small.txt"), "one\ntwo\n").unwrap();

    let result = tool
        .execute(r#"{"path": "small.txt", "offset": 999}"#)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_read_offset_zero_is_error() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("f.txt"), "hello").unwrap();

    let result = tool.execute(r#"{"path": "f.txt", "offset": 0}"#).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("1-indexed"));
}

#[tokio::test]
async fn test_read_png_returns_image_block() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let png_bytes: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE,
    ];
    std::fs::write(tmp.path().join("img.png"), png_bytes).unwrap();

    let result = tool.execute(r#"{"path": "img.png"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.image_blocks.len(), 1);
    assert_eq!(result.image_blocks[0].mime_type, "image/png");
    assert!(!result.image_blocks[0].data.is_empty());
    assert!(result.content.contains("image/png"));
}

#[tokio::test]
async fn test_read_text_file_has_no_image_blocks() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("hello.txt"), "hello world").unwrap();
    let result = tool.execute(r#"{"path": "hello.txt"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.image_blocks.is_empty());
    assert!(result.content.contains("hello world"));
}

// --- Magic-byte MIME detection ---

#[test]
fn test_detect_mime_by_magic_png() {
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
    assert_eq!(detect_mime_by_magic(PNG), Some("image/png"));
}

#[test]
fn test_detect_mime_by_magic_jpeg() {
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
    assert_eq!(detect_mime_by_magic(JPEG), Some("image/jpeg"));
}

#[test]
fn test_detect_mime_by_magic_gif89a() {
    assert_eq!(detect_mime_by_magic(b"GIF89a\x01"), Some("image/gif"));
}

#[test]
fn test_detect_mime_by_magic_gif87a() {
    assert_eq!(detect_mime_by_magic(b"GIF87a\x01"), Some("image/gif"));
}

#[test]
fn test_detect_mime_by_magic_webp() {
    let webp = b"RIFF\x20\x00\x00\x00WEBPVP8L";
    assert_eq!(detect_mime_by_magic(webp), Some("image/webp"));
}

#[test]
fn test_detect_mime_by_magic_text() {
    assert_eq!(detect_mime_by_magic(b"hello world"), None);
    assert_eq!(detect_mime_by_magic(b""), None);
}

#[tokio::test]
async fn test_magic_bytes_detect_png_no_extension() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    // Valid PNG magic bytes but no file extension
    let png_bytes: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52,
    ];
    std::fs::write(tmp.path().join("screenshot"), png_bytes).unwrap();
    let result = tool.execute(r#"{"path": "screenshot"}"#).await.unwrap();
    assert!(
        !result.is_error,
        "magic byte PNG should be detected: {}",
        result.content
    );
    assert!(
        result
            .image_blocks
            .iter()
            .any(|b| b.mime_type == "image/png")
    );
}

#[tokio::test]
async fn test_magic_bytes_detect_jpeg_wrong_extension() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    // JPEG magic bytes with .dat extension
    let jpeg_bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
    std::fs::write(tmp.path().join("photo.dat"), jpeg_bytes).unwrap();
    let result = tool.execute(r#"{"path": "photo.dat"}"#).await.unwrap();
    assert!(
        !result.is_error,
        "magic byte JPEG should be detected: {}",
        result.content
    );
    assert!(
        result
            .image_blocks
            .iter()
            .any(|b| b.mime_type == "image/jpeg")
    );
}

#[tokio::test]
async fn test_text_jpg_not_detected_as_image() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    // Text content with .jpg extension — magic bytes are not image bytes
    std::fs::write(tmp.path().join("not_image.jpg"), "hello world").unwrap();
    let result = tool.execute(r#"{"path": "not_image.jpg"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.image_blocks.is_empty(),
        "text file should not produce image block"
    );
    assert!(result.content.contains("hello world"));
}

// --- Truncation notice format ---

#[test]
fn test_truncation_byte_limit_includes_50kb_hint() {
    // Content > 50KB in multi-line format; should trigger byte truncation notice.
    // Each line is ~50 bytes; 1500 lines = ~75KB total, truncation happens by bytes.
    let line = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuv\n"; // 50 bytes
    let content: String = line.repeat(1500); // ~75KB
    let result = apply_read_truncation(&content, "f.txt", None, None).unwrap();
    assert!(
        result.contains("50KB limit"),
        "byte-truncated notice should include '50KB limit', got: {}",
        &result[result.len().saturating_sub(200)..]
    );
}

#[test]
fn test_truncation_user_limit_shows_more_lines() {
    // 100 lines, user requests 10 → "N more lines in file" notice.
    let content: String = (1..=100).map(|i| format!("line{}\n", i)).collect();
    let result = apply_read_truncation(&content, "f.txt", None, Some(10)).unwrap();
    assert!(
        result.contains("more lines in file"),
        "user-limit notice should say 'more lines in file', got: {}",
        &result[result.len().saturating_sub(200)..]
    );
}

#[test]
fn test_offset_read_returns_requested_window_from_large_file() {
    let content: String = (1..=20_000).map(|i| format!("line{i}\n")).collect();
    let result = apply_read_truncation(&content, "large.txt", Some(19_000), Some(5)).unwrap();
    assert!(result.starts_with("line19000\nline19001\nline19002\nline19003\nline19004"));
    assert!(!result.contains("line18999"));
    assert!(!result.contains("line19005\n"));
    assert!(result.contains("996 more lines in file. Use offset=19005 to continue."));
}

#[test]
fn test_read_limit_zero_is_rejected() {
    let err = apply_read_truncation("line1\nline2\n", "f.txt", None, Some(0)).unwrap_err();
    assert!(err.to_string().contains("limit must be at least 1"));
}

#[test]
fn test_offset_read_allows_exact_50kb_window() {
    let content = format!("skip\n{}", "a".repeat(DEFAULT_MAX_BYTES));
    let result = apply_read_truncation(&content, "large.txt", Some(2), None).unwrap();
    assert_eq!(result.len(), DEFAULT_MAX_BYTES);
    assert!(!result.contains("50KB limit"));
}

#[test]
fn test_offset_read_truncates_just_over_50kb_window() {
    let content = format!("skip\n{}\nnext\n", "a".repeat(DEFAULT_MAX_BYTES + 1));
    let result = apply_read_truncation(&content, "large.txt", Some(2), None).unwrap();
    assert!(result.contains("exceeds 50.0KB limit"));
    assert!(result.contains("sed -n '2p'"));
}

#[test]
fn test_offset_read_allows_exact_default_line_limit() {
    let content: String = (1..=2_001).map(|i| format!("line{i}\n")).collect();
    let result = apply_read_truncation(&content, "large.txt", Some(2), None).unwrap();
    assert!(result.starts_with("line2\n"));
    assert!(result.contains("line2001"));
    assert!(!result.contains("Use offset="));
}

#[test]
fn test_offset_read_truncates_one_line_over_default_line_limit() {
    let content: String = (1..=2_002).map(|i| format!("line{i}\n")).collect();
    let result = apply_read_truncation(&content, "large.txt", Some(2), None).unwrap();
    assert!(result.contains("line2001"));
    assert!(result.contains("Showing lines 2-2001 of 2002. Use offset=2002 to continue."));
    assert!(!result.contains("line2002\n"));
}

#[tokio::test]
async fn test_read_allows_file_at_10mib_limit() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("max.txt"), vec![b'a'; 10 * 1024 * 1024]).unwrap();
    let result = tool.execute(r#"{"path":"max.txt"}"#).await.unwrap();
    assert!(!result.is_error, "file at the limit should be readable");
}

#[tokio::test]
async fn test_read_rejects_file_over_10mib_limit() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(
        tmp.path().join("too-large.txt"),
        vec![b'a'; 10 * 1024 * 1024 + 1],
    )
    .unwrap();
    let result = tool.execute(r#"{"path":"too-large.txt"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("too large to read directly"));
}

#[tokio::test]
async fn test_read_allows_image_at_5mib_limit() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let mut bytes = vec![0_u8; 5 * 1024 * 1024];
    bytes[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    std::fs::write(tmp.path().join("max-image.bin"), bytes).unwrap();
    let result = tool.execute(r#"{"path":"max-image.bin"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.image_blocks.len(), 1);
}

#[tokio::test]
async fn test_read_rejects_image_over_5mib_limit() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let mut bytes = vec![0_u8; 5 * 1024 * 1024 + 1];
    bytes[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    std::fs::write(tmp.path().join("too-large-image.bin"), bytes).unwrap();
    let result = tool
        .execute(r#"{"path":"too-large-image.bin"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("too large to send inline"));
}

#[tokio::test]
async fn test_image_read_no_resize_note() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    // Minimal valid 1×1 PNG
    let png_bytes: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE,
    ];
    std::fs::write(tmp.path().join("small.png"), png_bytes).unwrap();
    let result = tool.execute(r#"{"path": "small.png"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(
        result.image_blocks.len(),
        1,
        "should return one image block"
    );
    assert!(
        !result.content.contains("resized"),
        "should not mention resize, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_read_empty_object_returns_actionable_error() {
    let (ws, sb, _tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error, "expected error, got: {}", result.content);
    assert!(
        result.content.contains("path"),
        "should mention 'path', got: {}",
        result.content
    );
    assert!(
        result.content.contains("Example"),
        "should include example, got: {}",
        result.content
    );
}

#[test]
fn test_read_description_includes_example() {
    let (ws, sb, _tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "read description should include Example, got: {}",
        def.description
    );
}
