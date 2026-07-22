//! Steps for `tui_table_safety.feature` — render markdown tables through the
//! real quecto-tui `Markdown` component and assert on sanitisation and
//! display-width handling. Width/truncation checks bind to the same production
//! `visible_width` / `truncate_to_width` utilities the table renderer uses.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::interface::ansi::sanitize_control;
use quecto_tui::interface::component::Component;
use quecto_tui::interface::components::markdown::Markdown;
use quecto_tui::interface::utils::{truncate_to_width, visible_width};

/// Wide render width so single-cell content never truncates for the sanitise
/// and width scenarios.
const WIDE: usize = 120;

/// Convert the `\xHH` byte escapes used in the Gherkin string literals into the
/// actual control bytes (the feature file text is literal `\x1b`, not ESC).
fn unescape(s: &str) -> String {
    // `\xHH` in the feature files only ever encodes ASCII control bytes, so
    // `byte as char` is exact for them; all other (incl. multi-byte) chars pass
    // through untouched.
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'x') {
            chars.next(); // consume 'x'
            if let (Some(a), Some(b)) = (chars.next(), chars.next()) {
                if let Ok(byte) = u8::from_str_radix(&format!("{a}{b}"), 16) {
                    out.push(byte as char);
                    continue;
                }
                out.push('\\');
                out.push('x');
                out.push(a);
                out.push(b);
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn render_table_with_cell(cell: &str) -> Vec<String> {
    // A minimal GFM table (header + delimiter + one data row) carrying the cell.
    let src = format!("| Col1 | Col2 |\n| --- | --- |\n| {cell} | data |\n");
    let mut md = Markdown::new(&src, 1);
    md.render(WIDE)
}

fn rendered(world: &TuiWorld) -> &[String] {
    world
        .tui_table_rendered
        .as_deref()
        .expect("the table must be rendered first")
}

fn raw(world: &TuiWorld) -> String {
    rendered(world).join("\n")
}

fn stripped(world: &TuiWorld) -> String {
    sanitize_control(&raw(world))
}

// ── Given ────────────────────────────────────────────────────────────────────

#[given(regex = r#"^a markdown table with cells containing "(.*)"$"#)]
fn table_with_cells(world: &mut TuiWorld, cell: String) {
    world.tui_table_cell = Some(unescape(&cell));
}

#[given("a markdown table where every cell is empty")]
fn table_all_empty(world: &mut TuiWorld) {
    world.tui_table_cell = Some(String::new());
}

#[given(regex = r#"^a table cell with "café" \(5 bytes, 4 display chars\)$"#)]
fn table_cell_cafe(world: &mut TuiWorld) {
    let cell = "café".to_string();
    assert_eq!(cell.len(), 5, "café is 5 UTF-8 bytes");
    assert_eq!(visible_width(&cell), 4, "café is 4 display columns");
    world.tui_table_cell = Some(cell);
}

#[given("markdown content with a heading, quote, list, unsafe link, and code fence")]
fn markdown_mixed_content(world: &mut TuiWorld) {
    world.tui_table_cell = Some(
        "# Release notes\n\n> quoted warning\n\n- first item\n\n[safe link](javascript:evil-title)\n\n```rs\u{1b}]0;evil-title\u{7}\nfn main() {}\n```\n"
            .to_string(),
    );
}

#[given(regex = r#"^markdown content with a code fence containing "([^"]*)"$"#)]
fn markdown_content_with_code_fence(world: &mut TuiWorld, code: String) {
    world.tui_table_cell = Some(format!("```rust\n{code}\n```\n"));
}

#[given("markdown content with a table containing a long cell value")]
fn markdown_table_with_long_cell_value(world: &mut TuiWorld) {
    world.tui_table_cell = Some(
        "| Name | Value |\n| --- | --- |\n| key | alpha-beta-gamma-delta-epsilon-zeta |\n"
            .to_string(),
    );
}

#[given("markdown content with a three column table whose first cell is a long value")]
fn markdown_three_column_table_with_long_first_cell(world: &mut TuiWorld) {
    world.tui_table_cell = Some(
        "| Path | Status | Notes |\n| --- | --- | --- |\n| alpha-beta-gamma-delta-epsilon-zeta | ok | fine |\n"
            .to_string(),
    );
}

// ── When ─────────────────────────────────────────────────────────────────────

#[when("the table is rendered")]
fn render_table(world: &mut TuiWorld) {
    let cell = world.tui_table_cell.clone().expect("cell content set");
    world.tui_table_rendered = Some(render_table_with_cell(&cell));
}

#[when("the table column widths are calculated")]
fn calc_column_widths(world: &mut TuiWorld) {
    let cell = world.tui_table_cell.clone().expect("cell content set");
    world.tui_table_rendered = Some(render_table_with_cell(&cell));
}

#[when("the cell is truncated to fit column width")]
fn truncate_cell(world: &mut TuiWorld) {
    let cell = world.tui_table_cell.clone().expect("cell content set");
    // Truncate to a width that keeps the whole cell (4) and to one that must
    // drop trailing display columns (3).
    let keep = truncate_to_width(&cell, 4, None);
    let cut = truncate_to_width(&cell, 3, None);
    world.tui_table_rendered = Some(vec![keep, cut]);
}

#[when(regex = r#"^the markdown is rendered at width (\d+)$"#)]
fn render_markdown_at_width(world: &mut TuiWorld, width: usize) {
    render_markdown_with_width(world, width);
}

#[when(regex = r#"^the markdown is rendered in a viewport that is (\d+) display columns wide$"#)]
fn render_markdown_in_viewport(world: &mut TuiWorld, width: usize) {
    world.tui_table_viewport_width = Some(width);
    render_markdown_with_width(world, width);
}

fn render_markdown_with_width(world: &mut TuiWorld, width: usize) {
    let src = world.tui_table_cell.clone().expect("markdown content set");
    let mut md = Markdown::new(&src, 1);
    world.tui_table_rendered = Some(md.render(width));
}

// ── Then ─────────────────────────────────────────────────────────────────────

#[then(regex = r#"^the displayed cell text should be "([^"]*)" without ANSI escapes$"#)]
fn cell_text_without_ansi(world: &mut TuiWorld, expected: String) {
    let plain = stripped(world);
    assert!(
        plain.contains(&expected),
        "rendered cell should contain {expected:?} after stripping: {plain:?}"
    );
    assert!(
        !raw(world).contains("\u{1b}[31m"),
        "the injected \\x1b[31m colour escape must be stripped from the render"
    );
}

#[then("no terminal control sequences should appear in output")]
fn no_control_sequences(world: &mut TuiWorld) {
    let raw = raw(world);
    // The injected cell escapes (SGR colour / reset) must not survive into the
    // cell content. (Theme styling on borders is applied by the renderer, but
    // the sanitiser guarantees the *injected* sequences are gone.)
    assert!(
        !raw.contains("\u{1b}[31m") && !raw.contains("\u{1b}[0mred"),
        "injected ANSI colour sequences must not appear in output: {raw:?}"
    );
}

#[then("the clear-screen sequence should not be present")]
fn clear_screen_absent(world: &mut TuiWorld) {
    assert!(
        !raw(world).contains("\u{1b}[2J") && !raw(world).contains("\u{1b}[H"),
        "the clear-screen / cursor-home sequences must be stripped"
    );
}

#[then("the cell should render as empty or safe text")]
fn cell_empty_or_safe(world: &mut TuiWorld) {
    // After stripping the injected control bytes the cell carries no escape
    // payload — the table still renders a bounded set of lines.
    let plain = stripped(world);
    assert!(
        !plain.contains('\u{1b}'),
        "no ESC bytes should remain in the sanitised render: {plain:?}"
    );
    assert!(
        !rendered(world).is_empty(),
        "the table should still render some lines"
    );
}

#[then("the OSC sequence should not be present")]
fn osc_absent(world: &mut TuiWorld) {
    assert!(
        !raw(world).contains("\u{1b}]8"),
        "the OSC 8 hyperlink sequence must be stripped from the render"
    );
    assert!(
        !raw(world).contains("evil.com"),
        "the OSC hyperlink target must not survive into the render"
    );
}

#[then(regex = r#"^the markdown output should contain "([^"]*)"$"#)]
fn markdown_output_contains(world: &mut TuiWorld, expected: String) {
    let plain = stripped(world);
    assert!(
        plain.contains(&expected),
        "markdown output should contain {expected:?}, got: {plain:?}"
    );
}

#[then("the markdown output should contain the complete long cell value")]
fn markdown_output_contains_complete_long_cell_value(world: &mut TuiWorld) {
    let raw = raw(world);
    let joined_visible_lines: String = rendered(world)
        .iter()
        .map(|line| sanitize_control(line).trim().to_string())
        .collect();
    assert!(
        joined_visible_lines.contains("alpha-beta-gamma-delta-epsilon-zeta"),
        "markdown output should contain the complete long cell value, got: {raw:?}"
    );
}

#[then(regex = r#"^the markdown output should not contain "([^"]*)"$"#)]
fn markdown_output_not_contains(world: &mut TuiWorld, unexpected: String) {
    let raw = raw(world);
    let plain = stripped(world);
    assert!(
        !raw.contains(&unexpected) && !plain.contains(&unexpected),
        "markdown output should not contain {unexpected:?}; raw={raw:?}, plain={plain:?}"
    );
}

#[then("no source OSC control sequences should appear in markdown output")]
fn no_source_osc_sequences_in_markdown(world: &mut TuiWorld) {
    let raw = raw(world);
    assert!(
        !raw.contains("\u{1b}]") && !raw.contains("\x1b]"),
        "markdown output should not include source OSC controls: {raw:?}"
    );
}

#[then("the code block body should remain visible with its gutter")]
fn code_block_body_remains_visible_with_gutter(world: &mut TuiWorld) {
    let expected = expected_code_body(world);
    let body_line = rendered_code_body_line(world, &expected);

    assert!(
        sanitize_control(body_line).contains(&format!("│ {expected}")),
        "code block body should keep the code gutter and text: {body_line:?}"
    );
}

#[then("the code block body should use the terminal default foreground and background")]
fn code_block_body_uses_terminal_default_theme(world: &mut TuiWorld) {
    let expected = expected_code_body(world);
    let body_line = rendered_code_body_line(world, &expected);
    let violations = code_body_active_sgr_violations(body_line, &expected);

    assert!(
        violations.is_empty(),
        "code block body must use terminal defaults, but found {violations}: {body_line:?}"
    );
}

fn expected_code_body(world: &TuiWorld) -> String {
    world
        .tui_table_cell
        .as_deref()
        .expect("markdown content set")
        .lines()
        .find(|line| !line.starts_with("```") && !line.is_empty())
        .expect("code body present")
        .to_string()
}

fn rendered_code_body_line<'a>(world: &'a TuiWorld, expected: &str) -> &'a str {
    let raw = raw(world);
    rendered(world)
        .iter()
        .find(|line| line.contains(expected))
        .map(String::as_str)
        .unwrap_or_else(|| panic!("code block body should render, got: {raw:?}"))
}

fn code_body_active_sgr_violations(body_line: &str, expected: &str) -> String {
    let code_start = body_line
        .find(expected)
        .unwrap_or_else(|| panic!("code body should be present: {body_line:?}"));
    let prefix = &body_line[..code_start];
    let active_prefix = prefix.rsplit("\u{1b}[0m").next().unwrap_or(prefix);
    let code_end = code_start + expected.len();
    let mut violations = Vec::new();

    if active_prefix.contains('\u{1b}') {
        violations.push("active SGR before code text");
    }
    if body_line[code_start..code_end].contains('\u{1b}') {
        violations.push("SGR inside code text");
    }

    violations.join(", ")
}

#[then("every markdown output line should fit within the viewport")]
fn every_markdown_output_line_fits_viewport(world: &mut TuiWorld) {
    let width = world
        .tui_table_viewport_width
        .expect("markdown viewport width set");
    let raw = raw(world);
    for line in rendered(world) {
        let plain = sanitize_control(line);
        assert!(
            visible_width(&plain) <= width,
            "markdown line must fit within {width} display columns, got {}: {plain:?}\nraw={raw:?}",
            visible_width(&plain)
        );
    }
}

#[then("the later table columns should stay aligned under their headers")]
fn later_table_columns_stay_aligned(world: &mut TuiWorld) {
    let raw = raw(world);
    let plain: Vec<String> = rendered(world)
        .iter()
        .map(|line| sanitize_control(line))
        .collect();
    let header = plain
        .iter()
        .find(|l| l.contains("Status"))
        .unwrap_or_else(|| panic!("header row with Status rendered, got: {raw:?}"));
    let status_col = header.find("Status").expect("Status offset");
    let notes_col = header.find("Notes").expect("Notes offset");
    let data_row = plain
        .iter()
        .find(|l| l.contains("ok"))
        .unwrap_or_else(|| panic!("data row with ok rendered, got: {raw:?}"));
    assert_eq!(
        data_row.find("ok"),
        Some(status_col),
        "Status cell must render under its header, got: {raw:?}"
    );
    assert_eq!(
        data_row.find("fine"),
        Some(notes_col),
        "Notes cell must render under its header, got: {raw:?}"
    );
}

#[then(regex = r#"^the cell text should be "([^"]*)" or stripped equivalent$"#)]
fn cell_text_or_stripped(world: &mut TuiWorld, expected: String) {
    let plain = stripped(world);
    assert!(
        plain.contains(&expected),
        "the visible link text {expected:?} should remain after stripping: {plain:?}"
    );
}

#[then("the column width should account for double-width CJK characters")]
fn column_accounts_cjk(world: &mut TuiWorld) {
    let cell = world.tui_table_cell.clone().unwrap();
    // Production measures with display width: 4 CJK chars = 8 columns, not 4.
    assert_eq!(
        visible_width(&cell),
        8,
        "CJK cell {cell:?} must measure 8 display columns"
    );
    let plain = stripped(world);
    assert!(
        plain.contains(&cell),
        "the CJK cell should render intact (column widened to fit): {plain:?}"
    );
}

#[then("the column should be at least 8 display columns wide")]
fn column_at_least_8(world: &mut TuiWorld) {
    let cell = world.tui_table_cell.clone().unwrap();
    let row = stripped(world)
        .lines()
        .find(|l| l.contains(&cell))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no rendered row contains the CJK cell"));
    assert!(
        visible_width(&row) >= 8,
        "the row holding the CJK cell must be at least 8 columns wide: {row:?}"
    );
}

#[then("the column width should account for double-width emoji")]
fn column_accounts_emoji(world: &mut TuiWorld) {
    let cell = world.tui_table_cell.clone().unwrap();
    // Two double-width emoji measure 4 display columns.
    assert_eq!(
        visible_width(&cell),
        4,
        "emoji cell {cell:?} must measure 4 display columns"
    );
    let plain = stripped(world);
    assert!(
        plain.contains(&cell),
        "the emoji cell should render intact: {plain:?}"
    );
}

#[then("no panic or division by zero should occur")]
fn no_panic(world: &mut TuiWorld) {
    // Reaching this step means render_markdown returned rather than panicking on
    // the all-empty table (the div-by-zero guard held).
    assert!(
        world.tui_table_rendered.is_some(),
        "the all-empty table must have rendered without panicking"
    );
}

#[then("the table should render without errors")]
fn table_renders_without_errors(world: &mut TuiWorld) {
    let lines = rendered(world);
    assert!(
        lines
            .iter()
            .all(|l| !l.contains('\u{1b}') || visible_width(l) <= WIDE),
        "rendered lines should stay within the render width"
    );
}

#[then("truncation should use display width not byte length")]
fn truncation_uses_display_width(world: &mut TuiWorld) {
    let out = rendered(world);
    let keep = &out[0];
    let cut = &out[1];
    let cell = world.tui_table_cell.clone().unwrap();
    // Fitting to 4 display columns keeps all of "café" (a byte-length cut to 4
    // would sever the 2-byte 'é').
    assert_eq!(
        visible_width(keep),
        4,
        "width-4 truncation must keep all 4 display columns: {keep:?}"
    );
    assert_eq!(keep, &cell, "width-4 truncation must keep the whole cell");
    // Fitting to 3 drops a display column, stays within width, and remains a
    // valid char-boundary prefix (display-width truncation, not a byte cut).
    assert!(
        visible_width(cut) <= 3,
        "width-3 truncation must not exceed 3 display columns: {cut:?}"
    );
    // Production appends a defensive SGR reset after a truncation; compare the
    // visible text only.
    let cut_plain = sanitize_control(cut);
    assert!(
        cell.starts_with(cut_plain.as_str()),
        "width-3 truncation must be a valid char-boundary prefix of the cell: {cut_plain:?}"
    );
    assert!(
        visible_width(cut) < visible_width(keep),
        "width-3 truncation must drop display columns vs width-4: {cut:?} vs {keep:?}"
    );
}
