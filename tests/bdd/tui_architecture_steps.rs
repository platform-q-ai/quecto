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
    assert!(Path::new(TUI_ROOT).join("domain").is_dir());
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
    assert!(Path::new(TUI_ROOT).join("application").is_dir());
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
    assert!(Path::new(TUI_ROOT).join("domain").is_dir());
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
    assert!(Path::new(TUI_ROOT).join("application").is_dir());
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
    assert!(Path::new(TUI_ROOT).join("infrastructure").is_dir());
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
        if path.extension().is_none_or(|ext| ext != "rs") {
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

#[given("a quecto-tui chat view with conversation history")]
fn given_tui_chat_view_with_history(world: &mut QuectoWorld) {
    world.tui_chat = Some(streaming_history_chat());
}

#[when("the chat is rendered twice without changes")]
fn when_tui_chat_rendered_twice(world: &mut QuectoWorld) {
    let chat = world
        .tui_chat
        .as_mut()
        .expect("TUI chat view should be initialized by the Given step");
    world.tui_viewport_before_stream = chat.render(TUI_SCROLLBACK_WIDTH);
    world.tui_viewport_after_stream = chat.render(TUI_SCROLLBACK_WIDTH);
}

#[then("both quecto-tui chat renders should be identical")]
fn then_tui_chat_renders_identical(world: &mut QuectoWorld) {
    assert_eq!(
        world.tui_viewport_before_stream, world.tui_viewport_after_stream,
        "an unchanged conversation must render identically across frames; \
         caching the concatenated buffer must not alter output (#757)"
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
    // Submit handling lives in the app_event_loop module.
    let content = std::fs::read_to_string("quecto-tui/src/interface/app_event_loop.rs")
        .expect("read quecto-tui app_event_loop source");
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
    let overlay = std::fs::read_to_string("quecto-tui/src/interface/select_overlay.rs")
        .expect("read quecto-tui select_overlay source");
    let theme = std::fs::read_to_string("quecto-tui/src/interface/theme.rs")
        .expect("read quecto-tui theme source");
    assert!(
        overlay.contains("build_resume_selector_overlay")
            && overlay.contains("SELECTOR_BORDER_WIDTH")
            && overlay.contains("apply_overlay_bg"),
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

#[then("the quecto-tui workflow widget should render as plain text matching the Quecto workflow")]
fn then_tui_workflow_widget_matches_quecto_plain_text(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    // The render composition lives in app_methods.rs, not app.rs.
    let app = std::fs::read_to_string("quecto-tui/src/interface/app_methods.rs")
        .expect("read app_methods source");
    assert!(
        bar.contains("render_widget")
            && bar.contains("Workflow")
            && bar.contains("→ Step")
            && bar.contains("✓ Workflow complete")
            && !bar.contains("BG_WORKFLOW_WIDGET"),
        "workflow widget should be plain Quecto-style text without a full-width yellow background"
    );
    assert!(
        app.contains("workflow_bar::render_widget")
            && app.contains("bottom.extend(workflow_bar::render_widget("),
        "app should render the workflow widget in the bottom section above the editor"
    );
}

#[then("the quecto-tui workflow widget should show workflow hotkey hints with toggle state")]
fn then_tui_workflow_widget_shows_hotkey_hints_with_toggle_state(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    assert!(
        bar.contains("Ctrl+Shift+A")
            && bar.contains("Ctrl+Shift+N")
            && bar.contains("auto:{auto}")
            && bar.contains("nudge:{nudge}"),
        "workflow widget should display active hotkey hints and live on/off toggle state"
    );
}

#[then("quecto-tui should not expose the Ctrl+Shift+W workflow overlay")]
fn then_tui_does_not_expose_ctrl_shift_w_workflow_overlay(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs").expect("read app source");
    assert!(
        !bar.contains("Ctrl+Shift+W") && !app.contains("Key::CtrlShift('w')"),
        "workflow UI should advertise only active Ctrl+Shift+A/N toggles, not the removed Ctrl+Shift+W overlay"
    );
    assert!(
        !app.contains("workflow_panel_open") && !app.contains("render_read_only_panel"),
        "read-only workflow overlay state and rendering should be removed"
    );
}

#[then("quecto-tui should not retain the dead OverlayStack overlay machinery")]
fn then_tui_drops_dead_overlay_stack(_world: &mut QuectoWorld) {
    let overlay = std::fs::read_to_string("quecto-tui/src/interface/overlay.rs")
        .expect("read quecto-tui overlay source");
    let app = std::fs::read_to_string("quecto-tui/src/interface/app.rs").expect("read app source");
    let app_methods = std::fs::read_to_string("quecto-tui/src/interface/app_methods.rs")
        .expect("read app_methods source");
    let event_loop = std::fs::read_to_string("quecto-tui/src/interface/app_event_loop.rs")
        .expect("read app_event_loop source");
    for needle in [
        "struct OverlayStack",
        "fn composite",
        "enum Anchor",
        "struct OverlayOptions",
        "struct OverlayEntry",
        "fn resolve_position",
    ] {
        assert!(
            !overlay.contains(needle),
            "dead OverlayStack machinery should be removed from overlay.rs: found `{needle}`"
        );
    }
    assert!(
        !app.contains("OverlayStack") && !app.contains("overlay_stack"),
        "app.rs should not hold or construct the dead overlay_stack field"
    );
    assert!(
        !app_methods.contains("overlay_stack"),
        "app_methods.rs should not composite via the dead overlay_stack"
    );
    assert!(
        !event_loop.contains("overlay_stack"),
        "app_event_loop.rs should not route input through the dead overlay_stack"
    );
}

#[then("quecto-tui should not keep tests that pin the dead OverlayStack machinery alive")]
fn then_tui_drops_dead_overlay_stack_tests(_world: &mut QuectoWorld) {
    // The dead machinery only survived `#![deny(dead_code)]` because the
    // overlay.rs `#[cfg(test)]` module exercised it; that resurrection must go.
    let overlay = std::fs::read_to_string("quecto-tui/src/interface/overlay.rs")
        .expect("read quecto-tui overlay source");
    for needle in ["OverlayStack::new()", ".composite(", "OverlayOptions"] {
        assert!(
            !overlay.contains(needle),
            "overlay.rs tests must not resurrect the dead OverlayStack machinery: found `{needle}`"
        );
    }
}

#[then("quecto-tui should keep the live splice_line overlay helpers")]
fn then_tui_keeps_splice_line_helpers(_world: &mut QuectoWorld) {
    let overlay = std::fs::read_to_string("quecto-tui/src/interface/overlay.rs")
        .expect("read quecto-tui overlay source");
    for needle in [
        "pub fn splice_line",
        "fn take_visible_chars",
        "fn skip_visible_chars",
    ] {
        assert!(
            overlay.contains(needle),
            "live overlay helper must be kept: missing `{needle}`"
        );
    }
}

#[then("quecto-tui should not retain the legacy workflow_bar render function")]
fn then_tui_drops_legacy_workflow_bar_render(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    for needle in [
        "pub fn render(",
        "fn render_stage_status_line",
        "fn pad_or_truncate_with_bg",
        "fn phase_bg",
        "fn short_step_label",
    ] {
        assert!(
            !bar.contains(needle),
            "legacy workflow_bar render path should be removed: found `{needle}`"
        );
    }
}

#[then("quecto-tui should not keep tests that pin the legacy workflow_bar render path alive")]
fn then_tui_drops_legacy_workflow_bar_render_tests(_world: &mut QuectoWorld) {
    // The legacy `render` + helpers only survived `#![deny(dead_code)]` because
    // workflow_bar_tests.rs called them; those tests must be deleted too.
    let tests =
        std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar_tests.rs")
            .expect("read workflow bar tests source");
    for needle in [
        "render(&state",
        "render_stage_status_line",
        "pad_or_truncate_with_bg",
        "phase_bg",
        "short_step_label",
    ] {
        assert!(
            !tests.contains(needle),
            "workflow_bar_tests.rs must not resurrect the legacy render path: found `{needle}`"
        );
    }
}

#[then("quecto-tui should keep the live workflow_bar render_widget path")]
fn then_tui_keeps_workflow_bar_render_widget(_world: &mut QuectoWorld) {
    let bar = std::fs::read_to_string("quecto-tui/src/interface/components/workflow_bar.rs")
        .expect("read workflow bar source");
    for needle in ["pub fn render_widget", "pub fn parse_workflow_event"] {
        assert!(
            bar.contains(needle),
            "live workflow_bar function must be kept: missing `{needle}`"
        );
    }
}

// ── issue-760: footer streaming indicator ──
//
// The footer's `is_streaming` flag was write-only: four production callers
// toggled it but `render()` never read it, so nothing showed. These scenarios
// pin the behaviour through the public render surface. We assert against the
// real `STREAMING_INDICATOR` glyph (a dedicated non-spinner marker) rather than
// a hardcoded literal so the step tracks the source of truth.

fn render_footer(streaming: bool) -> Vec<String> {
    use quecto_tui::interface::components::footer::Footer;
    let mut footer = Footer::new();
    footer.set_model("claude-sonnet-4-6");
    footer.set_streaming(streaming);
    footer.render(80)
}

fn streaming_glyph() -> &'static str {
    quecto_tui::interface::theme::STREAMING_INDICATOR
}

#[given("a quecto-tui footer marked as streaming")]
fn given_tui_footer_streaming(world: &mut QuectoWorld) {
    world.tui_footer_streaming_render = render_footer(true);
}

#[given("a quecto-tui footer that is idle")]
fn given_tui_footer_idle(world: &mut QuectoWorld) {
    world.tui_footer_idle_render = render_footer(false);
}

#[then("the quecto-tui footer should render a streaming indicator")]
fn then_tui_footer_renders_streaming_indicator(world: &mut QuectoWorld) {
    let joined = world.tui_footer_streaming_render.join("\n");
    assert!(
        joined.contains(streaming_glyph()),
        "footer must render a streaming indicator while streaming: {joined:?}"
    );
}

#[then("the quecto-tui footer should not render a streaming indicator")]
fn then_tui_footer_hides_streaming_indicator(world: &mut QuectoWorld) {
    let joined = world.tui_footer_idle_render.join("\n");
    assert!(
        !joined.contains(streaming_glyph()),
        "footer must not render a streaming indicator when idle: {joined:?}"
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

#[then("the TUI application layer should parse session stats payloads into typed values")]
fn then_tui_application_parses_session_stats(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("quecto-tui/src/application/session_payloads.rs")
        .expect("read TUI session payload parser");
    assert!(
        content.contains("pub struct SessionStats")
            && content.contains("pub fn parse_session_stats")
            && content.contains("context_usage"),
        "session stats JSON parsing should live in an application-layer typed value"
    );
}

#[then("the TUI application layer should validate resumed chat payloads into typed messages")]
fn then_tui_application_validates_resumed_chat(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("quecto-tui/src/application/session_payloads.rs")
        .expect("read TUI session payload parser");
    assert!(
        content.contains("pub enum ResumedChatMessage")
            && content.contains("pub enum ResumeMessagesError")
            && content.contains("pub fn parse_resumed_messages")
            && content.contains("Result<Vec<ResumedChatMessage>, ResumeMessagesError>")
            && content.contains("pub fn parse_resume_sessions"),
        "resumed chat/session-list JSON validation should live in application-layer typed values"
    );
}

#[then("the TUI should validate resumed messages before replacing chat history")]
fn then_tui_validates_resumed_messages_before_replacing_chat(_world: &mut QuectoWorld) {
    let parser = std::fs::read_to_string("quecto-tui/src/application/session_payloads.rs")
        .expect("read TUI session payload parser");
    assert!(
        parser.contains("ResumeMessagesError::MissingMessages")
            && parser.contains("ResumeMessagesError::MalformedMessages"),
        "resumed-message parser should distinguish missing and malformed messages payloads"
    );

    let methods = std::fs::read_to_string("quecto-tui/src/interface/app_methods.rs")
        .expect("read TUI app methods source");
    let body = rust_fn_body(&methods, "replace_chat_with_messages")
        .expect("expected replace_chat_with_messages in app_methods.rs");
    let parse_pos = body
        .find("session_payloads::parse_resumed_messages")
        .expect("replace_chat_with_messages should call the application-layer parser");
    let clear_pos = body
        .find("self.chat.clear()")
        .expect("replace_chat_with_messages should still clear chat after valid resume data");
    assert!(
        parse_pos < clear_pos
            && body.contains("Err(error)")
            && body.contains("Invalid resume payload"),
        "replace_chat_with_messages should parse/validate before clearing and report invalid payloads; body was:\n{body}"
    );
}

#[then("the TUI App methods should delegate session payload parsing to the application layer")]
fn then_tui_app_methods_delegate_session_payload_parsing(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("quecto-tui/src/interface/app_methods.rs")
        .expect("read TUI app methods source");
    assert!(
        content.contains("session_payloads::parse_session_stats")
            && content.contains("session_payloads::parse_resume_sessions")
            && content.contains("session_payloads::parse_resumed_messages"),
        "App session methods should call application-layer parsers instead of hand-parsing raw JSON"
    );
    for fn_name in [
        "update_footer_stats",
        "show_session_stats",
        "open_resume_selector",
        "replace_chat_with_messages",
    ] {
        let body = rust_fn_body(&content, fn_name)
            .unwrap_or_else(|| panic!("expected function {fn_name} in app_methods.rs"));
        assert!(
            !body.contains(".get(\"") && !body.contains("as_array") && !body.contains("as_u64"),
            "{fn_name} should not parse raw serde_json::Value fields directly; body was:\n{body}"
        );
    }
}

fn rust_fn_body(content: &str, name: &str) -> Option<String> {
    let marker = format!("fn {name}");
    let start = content.find(&marker)?;
    let open = content[start..].find('{')? + start;
    let mut depth = 0usize;
    for (offset, ch) in content[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(content[open + 1..open + offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[then("the TUI components layer should expose a shared ListNavigator")]
fn then_tui_components_expose_list_navigator(_world: &mut QuectoWorld) {
    let nav_path = Path::new(TUI_ROOT)
        .join("interface")
        .join("components")
        .join("list_navigator.rs");
    assert!(
        nav_path.is_file(),
        "shared list navigation helper must live at {}",
        nav_path.display()
    );
    let content = std::fs::read_to_string(&nav_path).expect("read ListNavigator source");
    assert!(
        content.contains("pub struct ListNavigator")
            && content.contains("pub fn move_next")
            && content.contains("pub fn move_previous")
            && content.contains("pub fn clamp"),
        "ListNavigator should own selected index movement and clamping"
    );
}

#[then(
    "slash autocomplete, files autocomplete, model selector, and select list should use ListNavigator"
)]
fn then_selector_components_use_list_navigator(_world: &mut QuectoWorld) {
    // model_selector and select_list own a ListNavigator directly. The two
    // autocomplete components delegate through the shared SuggestionList, which
    // in turn owns the ListNavigator — so navigation has a single home for all
    // four components without re-implementing window/wraparound logic.
    let read = |file: &str| {
        let path = Path::new(TUI_ROOT)
            .join("interface")
            .join("components")
            .join(file);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    };

    for file in ["model_selector.rs", "select_list.rs", "suggestion_list.rs"] {
        let content = read(file);
        assert!(
            content.contains("navigator: ListNavigator"),
            "{file} should store selected-index state in a ListNavigator field"
        );
        assert!(
            content.contains("navigator.move_next")
                && content.contains("navigator.move_previous")
                && content.contains(".visible_range("),
            "{file} should delegate movement and visible-window calculation to ListNavigator"
        );
    }

    // The autocomplete components share navigation via SuggestionList rather
    // than re-implementing it, so they must not hold their own navigator.
    for file in ["autocomplete.rs", "files_autocomplete.rs"] {
        let content = read(file);
        assert!(
            content.contains("list: SuggestionList"),
            "{file} should delegate list navigation to the shared SuggestionList"
        );
        assert!(
            content.contains("self.list.move_next")
                && content.contains("self.list.move_previous")
                && content.contains("self.list.visible_range"),
            "{file} should drive movement and visible-window calculation through SuggestionList"
        );
        assert!(
            !content.contains("navigator: ListNavigator"),
            "{file} should not hold its own ListNavigator now that SuggestionList owns it"
        );
    }
}

#[then("ListNavigator should own wraparound and visible-window selection behavior")]
fn then_list_navigator_owns_wraparound_and_window_behavior(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string(
        Path::new(TUI_ROOT)
            .join("interface")
            .join("components")
            .join("list_navigator.rs"),
    )
    .expect("read ListNavigator source");
    assert!(
        content.contains("pub fn selected")
            && content.contains("pub fn visible_range")
            && content.contains("saturating_sub"),
        "ListNavigator should expose selected index and visible-window computation"
    );
}

// ── #759: de-duplicate render compositing & near-identical components ──────

fn tui_read(rel: &str) -> String {
    let path = Path::new(TUI_ROOT).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[then("the quecto-tui render compositing should expose a composite_centered helper")]
fn then_compose_frame_has_helper(_world: &mut QuectoWorld) {
    let content = tui_read("interface/app_methods.rs");
    assert!(
        content.contains("fn composite_centered"),
        "compose_frame must extract a shared composite_centered helper for overlay splicing"
    );
}

#[then(
    "the quecto-tui resume, rewind, and model overlays should splice through composite_centered"
)]
fn then_overlays_use_composite_centered(_world: &mut QuectoWorld) {
    let content = tui_read("interface/app_methods.rs");
    // One definition + at least three call sites (resume, rewind, model).
    // Match the paren form so doc-comment mentions don't inflate the count.
    assert!(
        count_occurrences(&content, "composite_centered(") >= 4,
        "resume, rewind, and model overlays should all splice through composite_centered (def + 3 calls)"
    );
    // The hand-rolled splice loop should no longer be duplicated inline.
    assert!(
        count_occurrences(&content, "overlay::splice_line") <= 1,
        "splice_line should be invoked once inside composite_centered, not duplicated per overlay"
    );
}

#[then("the quecto-tui show_session_stats should delegate to update_footer_stats")]
fn then_show_session_stats_delegates(_world: &mut QuectoWorld) {
    let content = tui_read("interface/app_methods.rs");
    let body = rust_fn_body(&content, "show_session_stats")
        .expect("show_session_stats must exist in app_methods.rs");
    assert!(
        body.contains("update_footer_stats"),
        "show_session_stats should call update_footer_stats instead of duplicating footer logic"
    );
}

#[then("the quecto-tui footer context-usage update should appear once in app_methods")]
fn then_footer_update_appears_once(_world: &mut QuectoWorld) {
    let content = tui_read("interface/app_methods.rs");
    assert_eq!(
        count_occurrences(&content, "self.footer.update_context_usage("),
        1,
        "footer context-usage update logic must live in a single owner (update_footer_stats)"
    );
}

#[then("the quecto-tui components layer should expose a shared SuggestionList")]
fn then_components_expose_suggestion_list(_world: &mut QuectoWorld) {
    let path = Path::new(TUI_ROOT)
        .join("interface")
        .join("components")
        .join("suggestion_list.rs");
    assert!(
        path.is_file(),
        "shared SuggestionList component must live at {}",
        path.display()
    );
    let content = std::fs::read_to_string(&path).expect("read suggestion_list.rs");
    assert!(
        content.contains("pub struct SuggestionList"),
        "suggestion_list.rs must define a shared SuggestionList component"
    );
}

#[then("SuggestionList should own suggestions_match and set_suggestions")]
fn then_suggestion_list_owns_helpers(_world: &mut QuectoWorld) {
    let content = tui_read("interface/components/suggestion_list.rs");
    assert!(
        content.contains("fn suggestions_match") && content.contains("fn set_suggestions"),
        "SuggestionList should own suggestions_match and set_suggestions"
    );
    // The byte-identical copies must not remain in both component files.
    let autocomplete = tui_read("interface/components/autocomplete.rs");
    let files_autocomplete = tui_read("interface/components/files_autocomplete.rs");
    assert!(
        !autocomplete.contains("fn suggestions_match")
            && !files_autocomplete.contains("fn suggestions_match"),
        "suggestions_match must not be duplicated in autocomplete.rs / files_autocomplete.rs"
    );
}

#[then("slash autocomplete and files autocomplete should use SuggestionList")]
fn then_autocompletes_use_suggestion_list(_world: &mut QuectoWorld) {
    for file in ["autocomplete.rs", "files_autocomplete.rs"] {
        let content = tui_read(&format!("interface/components/{file}"));
        assert!(
            content.contains("SuggestionList"),
            "{file} should delegate to the shared SuggestionList component"
        );
    }
}

#[then("the quecto-tui chat_render should expose push_preview and push_header helpers")]
fn then_chat_render_has_helpers(_world: &mut QuectoWorld) {
    let content = tui_read("interface/components/chat_render.rs");
    assert!(
        content.contains("fn push_preview") && content.contains("fn push_header"),
        "chat_render.rs should extract shared push_preview / push_header helpers"
    );
}

#[then("the quecto-tui chat tool renderers should build previews and headers through the helpers")]
fn then_chat_renderers_use_helpers(_world: &mut QuectoWorld) {
    let content = tui_read("interface/components/chat_render.rs");
    assert!(
        count_occurrences(&content, "push_preview(") >= 4,
        "the repeated preview idiom should route through push_preview across the tool renderers"
    );
    // The issue calls out the header idiom repeating across 6 renderers, so a
    // partial refactor (only some converted) must not pass: def + 6 callsites.
    assert!(
        count_occurrences(&content, "push_header(") >= 6,
        "the repeated header idiom should route through push_header across all six tool renderers"
    );
}

#[then("the quecto-tui workflow_bar should expose exactly one phase-to-label map")]
fn then_workflow_bar_single_phase_map(_world: &mut QuectoWorld) {
    let content = tui_read("interface/components/workflow_bar.rs");
    let map_defs = count_occurrences(&content, "fn phase_display")
        + count_occurrences(&content, "fn phase_name");
    assert_eq!(
        map_defs, 1,
        "workflow_bar should collapse phase_display / phase_name into a single phase-to-label map"
    );
}

#[then("the quecto-tui workflow_bar should not keep the phase_label_for_widget forwarder")]
fn then_workflow_bar_no_forwarder(_world: &mut QuectoWorld) {
    let content = tui_read("interface/components/workflow_bar.rs");
    assert!(
        !content.contains("fn phase_label_for_widget"),
        "the trivial phase_label_for_widget forwarder should be removed"
    );
}

#[then("the quecto-tui client serialize-and-newline rule should appear once")]
fn then_client_serialize_once(_world: &mut QuectoWorld) {
    let content = tui_read("infrastructure/client.rs");
    // Counting a single shared helper definition is more robust than pinning the
    // literal `json.push('\n')` keystrokes: both senders must route through it.
    assert_eq!(
        count_occurrences(&content, "fn serialize_command"),
        1,
        "CommandSender::send and Client::send should share one serialize-and-newline helper"
    );
    assert_eq!(
        count_occurrences(&content, "json.push('\\n')"),
        1,
        "the serialize-and-newline rule should live in exactly one place"
    );
}

#[then("the quecto-tui markdown renderer should extract table and code-block flush handlers")]
fn then_markdown_extracts_handlers(_world: &mut QuectoWorld) {
    let content = tui_read("interface/components/markdown.rs");
    assert!(
        content.contains("fn flush_table") && content.contains("fn flush_code_block"),
        "render_markdown should extract per-block flush handlers (table / code-block)"
    );
}

#[then("the quecto-tui builtin command set should be the single source of truth")]
fn then_builtin_commands_single_source(_world: &mut QuectoWorld) {
    let content = tui_read("interface/app.rs");
    assert!(
        content.contains("fn builtin_commands"),
        "builtin_commands() must remain the single source of truth for slash commands"
    );
    // The previous triplication hand-listed the command help text in show_help.
    // That copy must be gone: show_help must derive its listing instead of
    // re-enumerating the `  /command   description` lines.
    let methods = tui_read("interface/app_methods.rs");
    for stale in ["/quit,/exit", "/workflow-nudge Toggle", "/resume <name>"] {
        assert!(
            !methods.contains(stale),
            "show_help must not re-enumerate the slash-command set ({stale}); derive it from builtin_commands()"
        );
    }
}

#[then("quecto-tui show_help and command dispatch should derive from builtin_commands")]
fn then_show_help_and_dispatch_derive(_world: &mut QuectoWorld) {
    let methods = tui_read("interface/app_methods.rs");
    assert!(
        methods.contains("builtin_commands("),
        "show_help should derive its listing from builtin_commands() rather than a hand-kept copy"
    );
    let event_loop = tui_read("interface/app_event_loop.rs");
    assert!(
        event_loop.contains("builtin_commands("),
        "the slash-command dispatch should derive from builtin_commands() rather than a hand-kept match"
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
