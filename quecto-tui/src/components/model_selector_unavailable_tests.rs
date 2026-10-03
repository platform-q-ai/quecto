//! #2435 review round 1 M2: a model the harness says cannot run is marked
//! with why, dimmed, and Enter on it is refused with the reason; the TUI's
//! fallback list matches the harness's built-in tables.

use super::*;
use crate::components::component::Component;

fn entry(id: &str, unavailable: Option<&str>) -> ModelEntry {
    ModelEntry {
        id: id.to_string(),
        provider: "openai-oauth".to_string(),
        auth: Some("oauth".to_string()),
        is_current: false,
        unavailable: unavailable.map(str::to_string),
    }
}

fn plain(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| crate::components::ansi::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn an_unavailable_model_is_marked_dimmed_and_refused() {
    let mut sel = ModelSelector::with_models(
        vec![
            entry("openai-oauth/mini", Some("refused-for-account: no ChatGPT")),
            entry("openai-oauth/sol", None),
        ],
        None,
    );
    let lines = sel.render(120);
    let text = plain(&lines);
    assert!(
        text.contains("(unavailable: refused for account)"),
        "the row carries a short tag: {text}"
    );
    assert!(
        !text.contains("no ChatGPT"),
        "the detail waits for Enter: {text}"
    );
    let row = lines
        .iter()
        .find(|line| line.contains("openai-oauth/mini"))
        .expect("the row is drawn");
    assert!(
        row.contains(&theme::dim("openai-oauth/mini")),
        "the label is dimmed: {row:?}"
    );
    sel.handle_input(&Key::Enter);
    assert_eq!(sel.take_result(), ModelSelectorResult::Pending, "refused");
    assert!(
        plain(&sel.render(120))
            .contains("openai-oauth/mini is unavailable: refused-for-account: no ChatGPT"),
        "the refusal says why"
    );
    sel.handle_input(&Key::Down);
    sel.handle_input(&Key::Enter);
    assert_eq!(
        sel.take_result(),
        ModelSelectorResult::Selected("openai-oauth/sol".into())
    );
}

/// Review round 2: the fallback list is the harness's built-in Anthropic and
/// OpenAI rows, each under the auth modes that offer it, as the harness
/// writes them to `model_selector_builtin_models.txt` (a harness test keeps
/// that file level with its tables).
#[test]
fn the_fallback_list_is_the_harness_built_in_rows() {
    let rows: Vec<&str> = include_str!("model_selector_builtin_models.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert!(rows.len() >= 22, "the built-in rows are listed: {rows:?}");
    let known: Vec<String> = known_models().into_iter().map(|m| m.id).collect();
    assert_eq!(known, rows);
}

#[test]
fn a_row_tag_names_each_reason_briefly() {
    assert_eq!(
        unavailable_tag("refused-for-account: no ChatGPT; missing-credential"),
        "refused for account, missing credential"
    );
    assert_eq!(unavailable_tag("not configured"), "not configured");
    assert_eq!(
        unavailable_tag("unsupported-transport: websocket-frames"),
        "unsupported transport"
    );
}
