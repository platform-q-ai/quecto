use super::*;

#[derive(Clone)]
struct FixtureItem {
    id: &'static str,
    label: &'static str,
    description: Option<&'static str>,
    metadata: Vec<&'static str>,
}

fn visible_ids(modal: &SelectableItemModal) -> Vec<&str> {
    modal
        .visible_indices
        .iter()
        .map(|idx| modal.rows[*idx].id.as_str())
        .collect()
}

#[test]
fn selectable_item_modal_ranks_id_and_label_matches_before_description_only_matches() {
    let mut modal = SelectableItemModal::builder()
        .items(vec![
            FixtureItem {
                id: "describe",
                label: "Describe",
                description: Some("metadata read assistance"),
                metadata: vec![],
            },
            FixtureItem {
                id: "read",
                label: "Read",
                description: Some("filesystem access"),
                metadata: vec![],
            },
            FixtureItem {
                id: "notes",
                label: "Notes",
                description: Some("read records from logs"),
                metadata: vec![],
            },
        ])
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_weights(SearchFieldWeights::tool_lookup())
        .build()
        .unwrap();

    for c in "read".chars() {
        modal.handle_input(&Key::Char(c));
    }

    assert_eq!(modal.selected_item(), Some("read"));

    let mut modal = SelectableItemModal::builder()
        .items(vec![
            FixtureItem {
                id: "file_reader",
                label: "File Viewer",
                description: Some("open file contents"),
                metadata: vec![],
            },
            FixtureItem {
                id: "filesystem",
                label: "Read Files",
                description: Some("open file contents"),
                metadata: vec![],
            },
            FixtureItem {
                id: "notes",
                label: "Notes",
                description: Some("read records from logs"),
                metadata: vec![],
            },
        ])
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_weights(SearchFieldWeights::tool_lookup())
        .build()
        .unwrap();
    for c in "read".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(modal.selected_item(), Some("filesystem"));
}

#[test]
fn selectable_item_modal_ranks_prefix_and_word_boundary_matches_before_scattered_matches() {
    let mut modal = SelectableItemModal::builder()
        .items(vec![
            FixtureItem {
                id: "workflow_event_bridge",
                label: "Workflow Event Bridge",
                description: Some("routes events"),
                metadata: vec![],
            },
            FixtureItem {
                id: "web_fetch",
                label: "Web Fetch",
                description: Some("fetch a URL"),
                metadata: vec![],
            },
            FixtureItem {
                id: "fetch_web",
                label: "Fetch Web",
                description: Some("network fetch"),
                metadata: vec![],
            },
        ])
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_weights(SearchFieldWeights::tool_lookup())
        .build()
        .unwrap();

    for c in "web".chars() {
        modal.handle_input(&Key::Char(c));
    }

    assert_eq!(
        visible_ids(&modal),
        vec!["web_fetch", "fetch_web", "workflow_event_bridge"]
    );
}

#[test]
fn selectable_item_modal_ranks_tool_name_matches_before_separate_description_mentions() {
    let mut modal = SelectableItemModal::builder()
        .items(vec![
            FixtureItem {
                id: "remote",
                label: "Remote",
                description: Some("web data from a cache fetch operation"),
                metadata: vec![],
            },
            FixtureItem {
                id: "web_fetch",
                label: "Web Fetch",
                description: Some("fetch a URL"),
                metadata: vec!["internet"],
            },
            FixtureItem {
                id: "bash",
                label: "Bash",
                description: Some("run commands"),
                metadata: vec!["shell"],
            },
        ])
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .search_weights(SearchFieldWeights::tool_lookup())
        .build()
        .unwrap();

    for c in "web fetch".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(modal.selected_item(), Some("web_fetch"));
    assert_eq!(visible_ids(&modal), vec!["web_fetch", "remote"]);
}

#[test]
fn selectable_item_modal_preserves_manually_moved_selection_while_filtering() {
    let mut modal = SelectableItemModal::builder()
        .items(vec![
            FixtureItem {
                id: "alpha",
                label: "Alpha Tool",
                description: Some("first tool"),
                metadata: vec![],
            },
            FixtureItem {
                id: "alpine",
                label: "Alpine Tool",
                description: Some("second tool"),
                metadata: vec![],
            },
        ])
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_weights(SearchFieldWeights::tool_lookup())
        .build()
        .unwrap();

    for c in "al".chars() {
        modal.handle_input(&Key::Char(c));
    }
    modal.handle_input(&Key::Down);
    assert_eq!(modal.selected_item(), Some("alpine"));

    modal.handle_input(&Key::Char('p'));

    assert_eq!(modal.selected_item(), Some("alpine"));
}

#[test]
fn selectable_item_modal_ranks_tool_alias_matches_for_common_terms() {
    let mut modal = SelectableItemModal::builder()
        .items(vec![
            FixtureItem {
                id: "remote_shell",
                label: "Remote Shell Notes",
                description: Some("documents command access"),
                metadata: vec![],
            },
            FixtureItem {
                id: "bash",
                label: "Bash",
                description: Some("run commands"),
                metadata: vec!["shell"],
            },
        ])
        .id(|item: &FixtureItem| item.id.to_string())
        .label(|item| item.label.to_string())
        .description(|item| item.description.map(str::to_string))
        .search_metadata(|item| item.metadata.iter().map(|s| (*s).to_string()).collect())
        .search_weights(SearchFieldWeights::tool_lookup())
        .build()
        .unwrap();
    for c in "shell".chars() {
        modal.handle_input(&Key::Char(c));
    }
    assert_eq!(visible_ids(&modal), vec!["bash", "remote_shell"]);
}
