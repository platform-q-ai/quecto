use super::*;

// Slice 5 (#1369): TUI acceptance for the canonical-runtime session. The
// production stack broadcasts REAL `subagent_state_changed` wire lines
// (built by `subagent_cascade` from the authoritative registries) while the
// canonical scripts run; these steps feed that exact line into the headless
// TUI render harness, so the environment layout the operator sees is proven
// end to end: canonical scripts → production registries → production wire
// serializer → production TUI parser and renderer.
// ===========================================================================

/// Drain the live-event broadcast and return the latest
/// `subagent_state_changed` line that satisfies `accept`, waiting up to 10s
/// for one to arrive.
fn latest_state_line(
    world: &mut QuectoWorld,
    accept: impl Fn(&serde_json::Value) -> bool,
) -> String {
    let rx = world
        .spawn_broadcast_rx
        .as_mut()
        .expect("live-event broadcast wired");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut latest: Option<String> = None;
    loop {
        match rx.try_recv() {
            Ok(line) => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                    if parsed["type"] == "subagent_state_changed" && accept(&parsed) {
                        latest = Some(line);
                    }
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                if let Some(line) = latest {
                    return line;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "no matching subagent_state_changed line was broadcast"
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => panic!("broadcast channel closed: {e}"),
        }
    }
}

/// Render `line` through the real TUI harness and store it on the world.
fn render_state_line(world: &mut QuectoWorld, line: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut h = rt.block_on(quecto_tui::shell::app::tui_harness::TuiHarness::sized(
        120, 40,
    ));
    {
        let _guard = rt.handle().enter();
        h.event(quecto_tui::protocol::client::Event::AgentStart);
        h.event_line(line);
    }
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

/// Drive the stored harness inside its runtime context.
fn drive<R>(
    world: &mut QuectoWorld,
    f: impl FnOnce(&mut quecto_tui::shell::app::tui_harness::TuiHarness) -> R,
) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("TUI harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

fn panel(world: &mut QuectoWorld) -> String {
    drive(world, |h| h.left_panel())
}

/// Non-empty panel row lines, excluding the bottom key-hint line.
fn panel_rows(panel: &str) -> Vec<String> {
    panel
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty() && !l.contains("⇥ pane"))
        .collect()
}

/// A row's text after the selection column and tree-stalk characters.
fn after_stalk(row: &str) -> &str {
    row.trim_start_matches(['▌', ' ', '│', '├', '└'])
}

/// Structural environment-row detection: the environment ref is the row's own
/// first label token (agent rows lead with the agent name).
fn is_environment_row(row: &str, env_ref: &str) -> bool {
    let label = after_stalk(row);
    label.starts_with(env_ref)
        && label[env_ref.len()..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ')
}

// --- When ---

#[when("the TUI renders the session's live subagent state")]
fn when_tui_renders_session(world: &mut QuectoWorld) {
    let line = latest_state_line(world, |_| true);
    render_state_line(world, &line);
}

// --- Then ---

#[then(expr = "the TUI panel should group {string} and {string} under environment row {string}")]
fn then_tui_groups_under_env(world: &mut QuectoWorld, a: String, b: String, env_ref: String) {
    let panel = panel(world);
    let rows = panel_rows(&panel);
    let env_idx = rows
        .iter()
        .position(|l| is_environment_row(l, &env_ref))
        .unwrap_or_else(|| panic!("no selectable environment row for {env_ref}:\n{panel}"));
    for id in [&a, &b] {
        let matches: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains(id.as_str()))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "agent {id} must appear exactly once (nested), never duplicated at the root:\n{panel}"
        );
        let idx = matches[0];
        assert!(
            idx > env_idx,
            "member {id} must be nested beneath the {env_ref} environment row:\n{panel}"
        );
        assert!(
            rows[idx].contains('├') || rows[idx].contains('└'),
            "member {id} must carry a nested tree connector:\n{}",
            rows[idx]
        );
    }
}

#[then(expr = "the TUI panel should show {string} as a flat row with environment badge {string}")]
fn then_tui_flat_badge_row(world: &mut QuectoWorld, id: String, env_ref: String) {
    let panel = panel(world);
    let rows = panel_rows(&panel);
    let row = rows
        .iter()
        .find(|l| l.contains(&id))
        .unwrap_or_else(|| panic!("no panel row for {id}:\n{panel}"));
    assert!(
        row.contains(&env_ref),
        "solo agent {id} must carry the {env_ref} badge on its own row:\n{panel}"
    );
    // The badge sits ON the agent's row: exactly one panel row mentions the
    // ref, so no separate selectable environment row exists for it.
    let ref_rows = rows.iter().filter(|l| l.contains(&env_ref)).count();
    assert_eq!(
        ref_rows, 1,
        "a solo environment must occupy exactly one row (the agent's own):\n{panel}"
    );
}

#[then(expr = "the TUI renders subagent {string} as exited")]
fn then_tui_renders_exited(world: &mut QuectoWorld, agent_id: String) {
    // Wait for the REAL pushed state line reporting the agent as exited, then
    // prove the TUI renders it in the exited style (dim name, not the
    // green/yellow live colours).
    let line = latest_state_line(world, |parsed| {
        parsed["subagents"].as_array().is_some_and(|subagents| {
            subagents.iter().any(|s| {
                s["agentId"].as_str() == Some(agent_id.as_str())
                    && s["status"].as_str() == Some("exited")
            })
        })
    });
    render_state_line(world, &line);
    let raw = drive(world, |h| h.full_frame_raw());
    let row = raw
        .lines()
        .find(|l| l.contains(&agent_id))
        .unwrap_or_else(|| panic!("no rendered row for {agent_id}:\n{raw}"))
        .to_string();
    // Inspect only the PANEL region of the raw row (left of the pane divider)
    // so main-pane styling on the same physical line cannot leak in.
    let panel_region = row
        .split('│')
        .next()
        .expect("split always yields a first segment")
        .to_string();
    assert!(
        panel_region.contains(&format!("\x1b[2m{agent_id}"))
            && !panel_region.contains("\x1b[32m")
            && !panel_region.contains("\x1b[33m"),
        "exited {agent_id} must render its name dim, not in a live colour: {panel_region:?}"
    );
}
