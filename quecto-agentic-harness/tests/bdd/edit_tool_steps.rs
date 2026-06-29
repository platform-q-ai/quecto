use super::*;

// Edit Tool Parity Steps — Issue #143
// ===========================================================================

/// File with CRLF byte sequences already embedded (not escaped).
#[given(expr = "a file {string} exists with CRLF bytes {string}")]
fn given_crlf_bytes_file(world: &mut QuectoWorld, filename: String, raw: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    // Interpret literal \r\n in the Gherkin string as actual CRLF bytes.
    let content = raw.replace("\\r\\n", "\r\n");
    std::fs::write(ws.join(&filename), content.as_bytes()).expect("write crlf file");
}

/// File with UTF-8 BOM prepended.
#[given(expr = "a file {string} exists with UTF-8 BOM and content {string}")]
fn given_bom_file(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let with_bom = format!("\u{FEFF}{}", content);
    std::fs::write(ws.join(&filename), with_bom.as_bytes()).expect("write bom file");
}

/// Execute edit with oldText containing a smart right single quote (U+2019).
#[when(expr = "the agent executes tool \"edit\" with smart-single-quote oldText on {string}")]
fn when_edit_smart_single_quote(world: &mut QuectoWorld, filename: String) {
    // oldText uses U+2019 RIGHT SINGLE QUOTATION MARK; file has ASCII apostrophe.
    let old_text = "it\u{2019}s a test";
    let args = serde_json::json!({
        "path": filename,
        "oldText": old_text,
        "newText": "it's replaced"
    })
    .to_string();
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("edit", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

/// Execute edit with oldText containing smart double quotes (U+201C / U+201D).
#[when(expr = "the agent executes tool \"edit\" with smart-double-quote oldText on {string}")]
fn when_edit_smart_double_quote(world: &mut QuectoWorld, filename: String) {
    // oldText uses U+201C/U+201D around "hello"; file has ASCII straight quotes.
    // Build via explicit JSON to ensure Unicode chars are embedded, not escaped.
    let old_text = format!("say {}hello{} now", '\u{201C}', '\u{201D}');
    let new_text = r#"say "goodbye" now"#;
    let args = format!(
        r#"{{"path":{},"oldText":{},"newText":{}}}"#,
        serde_json::to_string(&filename).unwrap(),
        serde_json::to_string(&old_text).unwrap(),
        serde_json::to_string(new_text).unwrap(),
    );
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("edit", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

/// Execute edit with oldText containing a Unicode en-dash (U+2013).
#[when(expr = "the agent executes tool \"edit\" with en-dash oldText on {string}")]
fn when_edit_en_dash(world: &mut QuectoWorld, filename: String) {
    // oldText uses U+2013 EN DASH; file has ASCII hyphen-minus.
    let old_text = "hello \u{2013} world";
    let args = serde_json::json!({
        "path": filename,
        "oldText": old_text,
        "newText": "replaced"
    })
    .to_string();
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("edit", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

/// Execute edit with oldText that has trailing spaces after "hello".
#[when(expr = "the agent executes tool \"edit\" with trailing-whitespace oldText on {string}")]
fn when_edit_trailing_whitespace(world: &mut QuectoWorld, filename: String) {
    // oldText has trailing spaces on the first line; file does not.
    let old_text = "hello   \nworld";
    let args = serde_json::json!({
        "path": filename,
        "oldText": old_text,
        "newText": "replaced"
    })
    .to_string();
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("edit", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

// --- Assertions ---

#[then(expr = "the file {string} should contain CRLF line endings")]
fn then_file_contains_crlf(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let bytes =
        std::fs::read(ws.join(&filename)).unwrap_or_else(|_| panic!("failed to read {}", filename));
    assert!(
        bytes.windows(2).any(|w| w == b"\r\n"),
        "expected file '{}' to contain CRLF line endings",
        filename
    );
}

#[then(expr = "the file {string} should not contain CRLF line endings")]
fn then_file_not_contains_crlf(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let bytes =
        std::fs::read(ws.join(&filename)).unwrap_or_else(|_| panic!("failed to read {}", filename));
    assert!(
        !bytes.windows(2).any(|w| w == b"\r\n"),
        "expected file '{}' to not contain CRLF line endings",
        filename
    );
}

#[then(expr = "the file {string} should start with a UTF-8 BOM")]
fn then_file_starts_with_bom(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let bytes =
        std::fs::read(ws.join(&filename)).unwrap_or_else(|_| panic!("failed to read {}", filename));
    assert!(
        bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
        "expected file '{}' to start with UTF-8 BOM (EF BB BF), got: {:02X?}",
        filename,
        &bytes[..bytes.len().min(6)]
    );
}
