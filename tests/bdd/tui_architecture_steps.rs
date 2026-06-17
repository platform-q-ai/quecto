use super::*;
use quecto_tui::interface::component::Component;
use quecto_tui::interface::components::chat::{Chat, ChatEntry};

const TUI_ROOT: &str = "quecto-tui/src";
const TUI_SCROLLBACK_WIDTH: usize = 80;
const TUI_SCROLLBACK_HEIGHT: usize = 10;

#[then(expr = "the quecto-tui source tree should contain layer {string}")]
fn then_tui_source_tree_contains_layer(_world: &mut QuectoWorld, layer: String) {
    let path = Path::new(TUI_ROOT).join(&layer);
    assert!(
        path.is_dir(),
        "quecto-tui layer directory must exist: {}",
        path.display()
    );
    assert!(
        path.join("mod.rs").is_file(),
        "quecto-tui layer must expose mod.rs: {}",
        path.display()
    );
}

#[then("the quecto-tui domain source should not contain runtime I/O patterns")]
fn then_tui_domain_no_runtime_io(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "domain",
        &[
            "std::fs::",
            "tokio::fs::",
            "std::env::",
            "dirs::",
            ".exists(",
        ],
    );
}

#[then("the quecto-tui application source should not contain runtime I/O patterns")]
fn then_tui_application_no_runtime_io(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "application",
        &[
            "std::fs::",
            "tokio::fs::",
            "std::env::",
            "dirs::",
            ".exists(",
        ],
    );
}

#[then("the quecto-tui domain source should not import outer layers")]
fn then_tui_domain_no_outer_layers(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "domain",
        &[
            "crate::application",
            "crate::infrastructure",
            "crate::interface",
            "super::application",
            "super::infrastructure",
            "super::interface",
        ],
    );
}

#[then("the quecto-tui application source should not import infrastructure or interface layers")]
fn then_tui_application_imports_only_inward(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "application",
        &[
            "crate::infrastructure",
            "crate::interface",
            "super::infrastructure",
            "super::interface",
        ],
    );
}

#[then("the quecto-tui infrastructure source should not import application or interface layers")]
fn then_tui_infrastructure_no_application_or_interface(_world: &mut QuectoWorld) {
    assert_no_tui_patterns(
        "infrastructure",
        &[
            "crate::application",
            "crate::interface",
            "super::application",
            "super::interface",
        ],
    );
}

#[then("the quecto-tui infrastructure layer should own runtime adapters")]
fn then_tui_infrastructure_owns_runtime_adapters(_world: &mut QuectoWorld) {
    for adapter in ["client", "process", "render", "signals", "terminal"] {
        let infrastructure_path = Path::new(TUI_ROOT)
            .join("infrastructure")
            .join(format!("{adapter}.rs"));
        let interface_path = Path::new(TUI_ROOT)
            .join("interface")
            .join(format!("{adapter}.rs"));
        assert!(
            infrastructure_path.is_file(),
            "runtime adapter must live in infrastructure: {}",
            infrastructure_path.display()
        );
        assert!(
            !interface_path.exists(),
            "runtime adapter must not live in interface: {}",
            interface_path.display()
        );
    }
}

#[then("every quecto-tui production Rust file should be under a Clean Architecture layer")]
fn then_every_tui_production_file_is_layered(_world: &mut QuectoWorld) {
    let misplaced = misplaced_tui_production_files();
    assert!(
        misplaced.is_empty(),
        "quecto-tui production Rust files must live under domain/, application/, infrastructure/, or interface/; misplaced: {misplaced:?}"
    );
}

#[then("the quecto-tui library root should expose only Clean Architecture layers")]
fn then_tui_library_root_exposes_only_layers(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("quecto-tui/src/lib.rs").expect("read quecto-tui lib.rs");
    let public_modules: Vec<_> = content
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("pub mod "))
        .map(|rest| rest.trim_end_matches(';'))
        .collect();
    assert_eq!(
        public_modules,
        ["application", "domain", "infrastructure", "interface"],
        "quecto-tui/src/lib.rs should match the main crate shape and expose only architecture layers"
    );
    assert!(
        !content.contains("#[path ="),
        "quecto-tui/src/lib.rs should not re-export interface internals with #[path] shims"
    );
}

#[then("the quecto-tui binary root should delegate to the interface layer")]
fn then_tui_binary_root_delegates_to_interface(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("quecto-tui/src/main.rs").expect("read quecto-tui main.rs");
    assert!(
        content.contains("quecto_tui::interface::cli") && content.lines().count() <= 10,
        "quecto-tui/src/main.rs should be a thin binary entrypoint delegating to interface::cli"
    );
}

#[then("the architecture test target should enforce quecto-tui Clean Architecture layers")]
fn then_architecture_test_enforces_tui_layers(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("tests/architecture.rs").expect("read architecture tests");
    assert!(
        content.contains("fn tui_architecture_layers_exist")
            && content.contains("fn tui_domain_has_no_outer_layer_imports")
            && content.contains("fn tui_application_has_no_infrastructure_or_interface_imports")
            && content.contains("fn tui_infrastructure_has_no_application_or_interface_imports")
            && content.contains("fn tui_runtime_adapters_live_in_infrastructure"),
        "tests/architecture.rs must enforce quecto-tui layer existence and dependency direction"
    );
}

#[then("the architecture test target should enforce quecto-tui runtime I/O boundaries")]
fn then_architecture_test_enforces_tui_runtime_io(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("tests/architecture.rs").expect("read architecture tests");
    assert!(
        content.contains("fn tui_inner_layers_have_no_runtime_io_calls")
            && content.contains("quecto-tui domain")
            && content.contains("quecto-tui application"),
        "tests/architecture.rs must enforce runtime I/O boundaries for quecto-tui inner layers"
    );
}

#[then("the architecture test target should enforce quecto-tui root file placement")]
fn then_architecture_test_enforces_tui_root_file_placement(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("tests/architecture.rs").expect("read architecture tests");
    assert!(
        content.contains("fn tui_production_files_live_inside_architecture_layers")
            && content.contains("TUI_ALLOWED_ROOT_RS")
            && content.contains("fn tui_lib_rs_exposes_only_architecture_layers")
            && content.contains("fn tui_main_rs_is_thin_interface_entrypoint"),
        "tests/architecture.rs must reject unlayered quecto-tui production source files and keep crate roots thin"
    );
}

fn misplaced_tui_production_files() -> Vec<String> {
    let mut misplaced = Vec::new();
    collect_misplaced_tui_rs_files(Path::new(TUI_ROOT), &mut misplaced);
    misplaced
}

fn collect_misplaced_tui_rs_files(dir: &Path, misplaced: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read quecto-tui src dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_misplaced_tui_rs_files(&path, misplaced);
            continue;
        }
        if !path.extension().is_some_and(|ext| ext == "rs") {
            continue;
        }
        let rel = path
            .strip_prefix(TUI_ROOT)
            .expect("strip quecto-tui src prefix")
            .to_string_lossy()
            .replace('\\', "/");
        let top = rel.split('/').next().unwrap_or_default();
        let in_layer = matches!(
            top,
            "domain" | "application" | "infrastructure" | "interface"
        );
        let allowed_root = !rel.contains('/') && matches!(rel.as_str(), "lib.rs" | "main.rs");
        if !in_layer && !allowed_root {
            misplaced.push(rel);
        }
    }
}

#[then("the BDD runner should execute TUI scenarios tagged wip or done")]
fn then_bdd_runner_executes_tui_wip_or_done(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("tests/bdd/main.rs").expect("read BDD runner");
    assert!(
        content.contains("tag_filter") && content.contains("wip") && content.contains("done"),
        "BDD runner must support executing selected @tui scenarios when they are tagged @wip or @done"
    );
}

#[given("a quecto-tui chat view is scrolled into history")]
fn given_tui_chat_view_scrolled_into_history(world: &mut QuectoWorld) {
    let mut chat = streaming_history_chat();
    chat.set_viewport_height(TUI_SCROLLBACK_HEIGHT);
    chat.scroll_up(15);
    world.tui_viewport_before_stream = chat.render(TUI_SCROLLBACK_WIDTH);
    world.tui_chat = Some(chat);
}

#[given("a quecto-tui chat view is scrolled beyond the oldest full page")]
fn given_tui_chat_view_scrolled_beyond_oldest_full_page(world: &mut QuectoWorld) {
    let mut chat = streaming_history_chat();
    chat.set_viewport_height(TUI_SCROLLBACK_HEIGHT);
    chat.scroll_up(10_000);
    world.tui_viewport_before_stream = chat.render(TUI_SCROLLBACK_WIDTH);
    world.tui_chat = Some(chat);
}

#[when("streaming assistant content extends the conversation")]
fn when_streaming_assistant_content_extends_conversation(world: &mut QuectoWorld) {
    let chat = world
        .tui_chat
        .as_mut()
        .expect("TUI chat view should be initialized by the Given step");
    chat.append_token("\nnew streamed line 1\nnew streamed line 2\nnew streamed line 3");
    world.tui_viewport_after_stream = chat.render(TUI_SCROLLBACK_WIDTH);
}

#[then("the quecto-tui chat viewport should keep showing the same historical lines")]
fn then_tui_chat_viewport_keeps_showing_same_history(world: &mut QuectoWorld) {
    assert_eq!(
        world.tui_viewport_after_stream, world.tui_viewport_before_stream,
        "streaming output should not move a user-scrolled TUI viewport toward the bottom"
    );
}

fn streaming_history_chat() -> Chat {
    let mut chat = Chat::new();
    for i in 0..30 {
        chat.add_entry(ChatEntry::User {
            text: format!("history line {i}"),
        });
    }
    chat.append_token("initial streamed response");
    chat
}

#[then("the quecto-tui chat viewport should still show a full historical page")]
fn then_tui_chat_viewport_still_shows_full_historical_page(world: &mut QuectoWorld) {
    assert_eq!(
        world.tui_viewport_after_stream.len(),
        TUI_SCROLLBACK_HEIGHT,
        "scrollback should clamp to a full page instead of shrinking to blank lines"
    );
    assert_eq!(
        world.tui_viewport_after_stream, world.tui_viewport_before_stream,
        "streaming output should not disturb the oldest full historical page"
    );
}

#[then(expr = "the quecto-tui slash autocomplete should include command {string}")]
fn then_tui_slash_autocomplete_includes_command(_world: &mut QuectoWorld, command: String) {
    let content = std::fs::read_to_string("quecto-tui/src/interface/app.rs")
        .expect("read quecto-tui app source");
    assert!(
        content.contains(&format!("name: \"{command}\".into()")),
        "quecto-tui builtin slash command list should include /{command}"
    );
}

#[then("quecto-tui should reject unknown slash commands before sending a prompt")]
fn then_tui_rejects_unknown_slash_commands(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("quecto-tui/src/interface/app.rs")
        .expect("read quecto-tui app source");
    assert!(
        content.contains("reject_unknown_slash_command"),
        "quecto-tui should route unknown slash commands to a local rejection helper instead of sending them as prompts"
    );
}

#[then("the UDS protocol should support listing sessions")]
fn then_uds_protocol_supports_listing_sessions(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("src/interface/cli/protocol.rs").expect("read UDS protocol source");
    assert!(
        content.contains("ListSessions") && content.contains("list_sessions"),
        "UDS protocol should include list_sessions support"
    );
}

#[then("the UDS protocol should support resuming a session")]
fn then_uds_protocol_supports_resuming_session(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("src/interface/cli/protocol.rs").expect("read UDS protocol source");
    assert!(
        content.contains("ResumeSession") && content.contains("resume_session"),
        "UDS protocol should include resume_session support"
    );
}

#[then("the quecto-tui resume selector should render with an opaque border")]
fn then_tui_resume_selector_has_opaque_border(_world: &mut QuectoWorld) {
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs")
        .expect("read quecto-tui app source");
    let theme = std::fs::read_to_string("quecto-tui/src/interface/theme.rs")
        .expect("read quecto-tui theme source");
    assert!(
        app.contains("build_resume_selector_overlay")
            && app.contains("RESUME_SELECTOR_BORDER_WIDTH")
            && app.contains("apply_overlay_bg"),
        "resume selector should be rendered as a padded opaque overlay instead of raw text over chat history"
    );
    assert!(
        theme.contains("BG_OVERLAY") && theme.contains("apply_overlay_bg"),
        "theme should expose an opaque overlay background for modal readability"
    );
}

#[then("quecto-tui should not render a separate workflow header bar")]
fn then_tui_does_not_render_workflow_header_bar(_world: &mut QuectoWorld) {
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs").expect("read app source");
    assert!(
        !app.contains("workflow_bar::render(&workflow_bar_state"),
        "workflow UI should only render in the bottom widget area, not as a top header bar"
    );
}

#[then("the quecto-tui workflow widget should render as plain text matching the Pi extension")]
fn then_tui_workflow_widget_matches_pi_plain_text(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs").expect("read app source");
    assert!(
        bar.contains("render_widget")
            && bar.contains("Workflow")
            && bar.contains("→ Step")
            && bar.contains("✓ Workflow complete")
            && !bar.contains("BG_WORKFLOW_WIDGET"),
        "workflow widget should be plain Pi-style text without a full-width yellow background"
    );
    assert!(
        app.contains("workflow_bar::render_widget")
            && app.contains("bottom.extend(workflow_widget_lines)"),
        "app should render the workflow widget in the bottom section above the editor"
    );
}

#[then("the quecto-tui workflow widget should show workflow hotkey hints with toggle state")]
fn then_tui_workflow_widget_shows_hotkey_hints_with_toggle_state(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    assert!(
        bar.contains("Ctrl+Shift+W")
            && bar.contains("Ctrl+Shift+A")
            && bar.contains("Ctrl+Shift+N")
            && bar.contains("auto:{auto}")
            && bar.contains("nudge:{nudge}"),
        "workflow widget should display hotkey hints and live on/off toggle state"
    );
}

#[then("the quecto-tui workflow panel should render the Pi workflow checklist in read-only mode")]
fn then_tui_workflow_panel_matches_pi_read_only(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs").expect("read app source");
    assert!(
        bar.contains("render_read_only_panel")
            && bar.contains("Quecto Dev Workflow")
            && bar.contains("BDD/TDD Red → Green → Refactor")
            && bar.contains("↑↓ navigate")
            && bar.contains("Esc close"),
        "workflow panel should mirror the Pi WorkflowChecklist render in read-only mode"
    );
    assert!(
        app.contains("workflow_panel_open") && app.contains("render_read_only_panel"),
        "app should open and render the read-only workflow checklist panel"
    );
}

#[then("quecto-tui should not swallow all keys when the workflow panel is open")]
fn then_tui_workflow_panel_does_not_swallow_toggles(_world: &mut QuectoWorld) {
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs").expect("read app source");
    let close_block = app
        .split("// If the read-only workflow panel is active")
        .nth(1)
        .expect("workflow panel key handling block should exist");
    assert!(
        close_block.contains("workflow_panel_open = false"),
        "workflow panel should close on Esc/Ctrl+C/Ctrl+Shift+W"
    );
    assert!(
        !close_block.contains("if self.workflow_panel_open {\n            if matches!(key,"),
        "workflow panel must not return early for every key while open; toggles must still reach handlers"
    );
    assert!(
        close_block.contains("Workflow toggles (Ctrl+Shift+A/N) still work"),
        "workflow panel key handling should explicitly allow Ctrl+Shift+A/N toggles"
    );
}

#[then("the TUI architecture feature should not contain pending scenarios")]
fn then_tui_architecture_feature_not_pending(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("tests/features/tui_clean_architecture.feature")
        .expect("read TUI architecture feature");
    assert!(
        !content.contains("@pending"),
        "TUI architecture feature must remain executable and not be marked @pending"
    );
}

fn assert_no_tui_patterns(layer: &str, forbidden: &[&str]) {
    let dir = Path::new(TUI_ROOT).join(layer);
    let mut files = Vec::new();
    collect_tui_rs_files(&dir, &mut files);
    assert!(
        !files.is_empty(),
        "quecto-tui {layer} layer must contain Rust source files"
    );

    for file_content in &files {
        let (file_path, _) = file_content
            .split_once(":\n")
            .expect("split path from file content");
        for line in file_content.lines().skip(1) {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") || trimmed.starts_with("//!") {
                continue;
            }
            for pattern in forbidden {
                assert!(
                    !trimmed.contains(pattern),
                    "quecto-tui {layer} architecture violation in {file_path}: {trimmed}; forbidden pattern: {pattern}"
                );
            }
        }
    }
}

fn collect_tui_rs_files(dir: &Path, files: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read TUI layer dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_tui_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read TUI source file");
            files.push(format!("{}:\n{}", path.display(), content));
        }
    }
}
