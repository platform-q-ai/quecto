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
        text.contains("(unavailable: refused-for-account: no ChatGPT)"),
        "{text}"
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

/// Review round 1 nit: the fallback list is the harness's built-in OpenAI
/// and Anthropic tables, read from their source.
#[test]
fn the_fallback_list_matches_the_harness_built_in_tables() {
    let tables = include_str!(
        "../../../quecto-agentic-harness/src/infrastructure/model_registry_builtin_tables.rs"
    );
    let ids_in = |table: &str| -> Vec<String> {
        let start = tables
            .find(&format!("const {table}: &[BuiltinModel] = &["))
            .unwrap_or_else(|| panic!("the {table} table"));
        let body = &tables[start..];
        let body = &body[..body.find("];").expect("the table ends")];
        body.lines()
            .filter_map(|line| line.trim().strip_prefix("builtin(\""))
            .map(|rest| rest.split('"').next().unwrap().to_string())
            .collect()
    };
    let mut expected = Vec::new();
    for (vendor, table) in [("anthropic", "ANTHROPIC"), ("openai", "OPENAI")] {
        let ids = ids_in(table);
        assert!(!ids.is_empty(), "{table} lists models");
        for id in ids {
            for auth in ["api", "oauth"] {
                expected.push(format!("{vendor}-{auth}/{id}"));
            }
        }
    }
    let mut known: Vec<String> = known_models().into_iter().map(|m| m.id).collect();
    known.sort();
    expected.sort();
    assert_eq!(known, expected);
}
