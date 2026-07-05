#![allow(private_interfaces)]

use cucumber::{World, given, then, when};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tempfile::TempDir;

mod feature_preprocess;

// Opaque Debug wrapper for the headless TUI render harness (#805). The harness
// holds a live `App` (and background tokio tasks) and isn't `Debug`, so wrap it
// to satisfy the derived `Debug`/`Default` on `TuiWorld`.
pub struct TuiParityHarness(pub quecto_tui::interface::app::tui_harness::TuiHarness);

impl std::fmt::Debug for TuiParityHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<TuiParityHarness>")
    }
}

// Opaque Debug wrappers for TUI components that aren't `Debug` themselves, so
// they can live in the derived-`Debug` `TuiWorld`. `DerefMut` lets step code
// call their inherent methods directly.
pub struct DebugStdinBuffer(pub quecto_tui::interface::stdin_buffer::StdinBuffer);
impl std::fmt::Debug for DebugStdinBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<StdinBuffer>")
    }
}
impl std::ops::Deref for DebugStdinBuffer {
    type Target = quecto_tui::interface::stdin_buffer::StdinBuffer;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DebugStdinBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct DebugEditor(pub quecto_tui::interface::components::editor::Editor);
impl std::fmt::Debug for DebugEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<Editor>")
    }
}
impl std::ops::Deref for DebugEditor {
    type Target = quecto_tui::interface::components::editor::Editor;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DebugEditor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Default, World)]
pub struct TuiWorld {
    pub stdout: String,
    pub stderr: String,
    /// Temp dirs kept alive for scenarios that need real filesystem state.
    pub _extra_temp_dirs: Vec<TempDir>,
    /// TUI scrollback BDD: chat view under test.
    pub tui_chat: Option<quecto_tui::interface::components::chat::Chat>,
    /// TUI @files BDD: file-mention autocomplete under test.
    pub tui_files_autocomplete:
        Option<quecto_tui::interface::components::files_autocomplete::FilesAutocomplete>,
    /// TUI @files BDD: last consumed background-load request.
    pub tui_files_load_requested: bool,
    /// TUI sub-agent session-parity BDD (#805): tokio runtime backing the
    /// headless render harness (its background tasks need a live runtime).
    pub tui_parity_rt: Option<tokio::runtime::Runtime>,
    /// TUI sub-agent session-parity BDD (#805): the headless render harness.
    pub tui_parity: Option<TuiParityHarness>,
    /// TUI observer-marker BDD (#966): currently tracked sub-agents and whether
    /// each is read-only, used to exercise selective departure.
    pub tui_expected_subagents: Vec<(String, bool)>,
    /// TUI idle-efficiency BDD (#978): spinner frame captured before the idle tick.
    pub tui_idle_spinner_frame: Option<usize>,
    /// TUI streaming-stability BDD (#972): frames painted during a token burst.
    pub tui_render_count: Option<usize>,
    /// TUI idle-efficiency BDD (#978): branch name expected after a switch.
    pub tui_idle_expected_branch: Option<String>,
    /// TUI idle-efficiency BDD (#978): whether kitty fallback detection completed.
    pub tui_idle_fallback_done: Option<bool>,
    /// TUI UDS client defensive-bounds BDD (#982): socket/client state.
    pub tui_defence_stream: Option<tui_uds_client_defence_steps::TuiDefenceStream>,
    /// The sub-agent id currently being viewed (#828): captured on select so
    /// backfill/assertion steps route to the right session, not a literal id.
    pub tui_viewed_agent: Option<String>,
    /// TUI scrollback BDD: viewport captured after streaming growth.
    pub tui_viewport_after_stream: Vec<String>,
    // --- TUI markdown table safety BDD ---
    pub tui_table_rendered: Option<Vec<String>>,
    pub tui_table_cell: Option<String>,
    // --- TUI stdin buffer cap BDD ---
    pub tui_stdin_buffer: Option<DebugStdinBuffer>,
    pub tui_stdin_last_feed_ok: Option<bool>,
    pub tui_stdin_fed_total: usize,
    pub tui_stdin_drained: Option<Vec<Vec<u8>>>,
    // --- TUI editor border replication BDD ---
    pub tui_editor: Option<DebugEditor>,
    pub tui_editor_renders: Vec<Vec<String>>,
    pub tui_render_full: Option<String>,
    pub tui_render_diff: Option<String>,
    /// TUI Esc-abort-recovery BDD: command lines drained from the headless harness.
    pub tui_last_commands: Vec<String>,
    // ── TUI PID-safety BDD (`tui_pid_safety.feature`) ──────────────────
    pub tui_pid_input: Option<u32>,
    pub tui_pid_result: Option<Result<i32, String>>,
    pub tui_pid_group_target: Option<i32>,
    // ── TUI stdin-retry BDD (`tui_stdin_retry.feature`) ────────────────
    pub tui_stdin_fragments: Vec<Vec<u8>>,
    pub tui_stdin_emitted: Vec<Vec<u8>>,
    pub tui_stdin_pending_after: bool,
    pub tui_stdin_force_drained: bool,
    pub tui_stdin_leftover: Option<usize>,
    // ── TUI foundation BDD (`tui_foundation.feature`) ──────────────────
    pub tui_foundation_disconnect: bool,
    pub tui_foundation_notification: String,
    pub tui_foundation_render_was_err: bool,
}

#[derive(Debug)]
struct ScenarioShardEntry {
    feature: String,
    scenario: String,
    weight: u64,
}

fn scenario_weight(tags: &[String], steps: &[String], feature: &str, scenario: &str) -> u64 {
    let lower_feature = feature.to_ascii_lowercase();
    let lower_scenario = scenario.to_ascii_lowercase();
    let mut weight = 1_u64 + steps.len() as u64;
    if tags.iter().any(|t| t == "manual-real-llm")
        || lower_feature.contains("real llm")
        || lower_scenario.contains("real llm")
    {
        weight += 200;
    }
    if tags.iter().any(|t| t == "provider-smoke") || lower_feature.contains("provider smoke") {
        weight += 80;
    }
    if lower_feature.contains("tui") || lower_scenario.contains("tui") {
        weight += 25;
    }
    weight
}

fn discover_scenarios(features_dir: &str) -> Vec<ScenarioShardEntry> {
    let mut files = Vec::new();
    let root = PathBuf::from(features_dir);
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "feature") {
            files.push(path);
        }
    }
    files.sort();

    let mut out = Vec::new();
    for path in files {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut feature_name = String::new();
        let mut pending_tags: Vec<String> = Vec::new();
        let mut current_scenario: Option<String> = None;
        let mut current_tags: Vec<String> = Vec::new();
        let mut current_steps: Vec<String> = Vec::new();

        let flush_current = |out: &mut Vec<ScenarioShardEntry>,
                             feature_name: &String,
                             current_scenario: &mut Option<String>,
                             current_tags: &mut Vec<String>,
                             current_steps: &mut Vec<String>| {
            if let Some(scenario_name) = current_scenario.take() {
                let weight =
                    scenario_weight(current_tags, current_steps, feature_name, &scenario_name);
                out.push(ScenarioShardEntry {
                    feature: feature_name.clone(),
                    scenario: scenario_name,
                    weight,
                });
                current_tags.clear();
                current_steps.clear();
            }
        };

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("Feature:") {
                feature_name = rest.trim().to_string();
                continue;
            }
            if line.starts_with('@') {
                pending_tags.extend(
                    line.split_whitespace()
                        .filter_map(|t| t.strip_prefix('@').map(str::to_string)),
                );
                continue;
            }
            if let Some(rest) = line
                .strip_prefix("Scenario:")
                .or_else(|| line.strip_prefix("Scenario Outline:"))
            {
                flush_current(
                    &mut out,
                    &feature_name,
                    &mut current_scenario,
                    &mut current_tags,
                    &mut current_steps,
                );
                current_scenario = Some(rest.trim().to_string());
                current_tags = std::mem::take(&mut pending_tags);
                continue;
            }
            if current_scenario.is_some() && !line.is_empty() {
                current_steps.push(line.to_string());
            }
        }
        flush_current(
            &mut out,
            &feature_name,
            &mut current_scenario,
            &mut current_tags,
            &mut current_steps,
        );
    }
    out
}

fn stable_hash(feature: &str, scenario: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    feature.hash(&mut hasher);
    scenario.hash(&mut hasher);
    hasher.finish()
}

fn build_shard_plan(
    features_dir: &str,
    shard_total: u64,
) -> std::collections::HashMap<(String, String), u64> {
    let mut scenarios = discover_scenarios(features_dir);
    scenarios.sort_by(|a, b| {
        b.weight.cmp(&a.weight).then_with(|| {
            stable_hash(&a.feature, &a.scenario).cmp(&stable_hash(&b.feature, &b.scenario))
        })
    });

    let mut loads = vec![0_u64; shard_total as usize];
    let mut plan = std::collections::HashMap::new();
    for s in scenarios {
        let key_hash = stable_hash(&s.feature, &s.scenario);
        let mut best_idx = 0_u64;
        let mut best_load = u64::MAX;
        let mut best_tie = u64::MAX;
        for idx in 0..shard_total {
            let load = loads[idx as usize];
            let tie = key_hash ^ idx;
            if load < best_load || (load == best_load && tie < best_tie) {
                best_load = load;
                best_tie = tie;
                best_idx = idx;
            }
        }
        loads[best_idx as usize] += s.weight;
        plan.insert((s.feature, s.scenario), best_idx);
    }
    plan
}

mod mouse_selection_steps;
mod tui_app_behaviors_steps;
mod tui_autocomplete_steps;
mod tui_border_replication_steps;
mod tui_chat_spacing_steps;
mod tui_cold_start_steps;
mod tui_ctrl_c_clear_steps;
mod tui_ctrl_d_exit_steps;
mod tui_esc_abort_recovery_steps;
mod tui_file_mention_steps;
mod tui_foundation_steps;
mod tui_idle_efficiency_steps;
mod tui_new_reset_context_steps;
mod tui_pid_safety_steps;
mod tui_stdin_buffer_cap_steps;
mod tui_stdin_retry_steps;
mod tui_streaming_stability_steps;
mod tui_subagent_first_layout_steps;
mod tui_subagent_parity_steps;
mod tui_subagent_readonly_marker_steps;
mod tui_table_safety_steps;
mod tui_terminal_restore_steps;
mod tui_uds_client_defence_steps;

fn main() {
    let tag_filter = std::env::var("QUECTO_TAG").ok();
    let shard_index = std::env::var("QUECTO_BDD_SHARD_INDEX")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let shard_total = std::env::var("QUECTO_BDD_SHARD_TOTAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let shard = match (shard_index, shard_total) {
        (Some(i), Some(t)) if t > 0 && i < t => Some((i, t)),
        _ => None,
    };
    let shard_plan = shard.map(|(_, total)| build_shard_plan("tests/features", total));

    let (_stripped_dir, stripped_features_path) =
        feature_preprocess::stripped_features_tempdir(std::path::Path::new("tests/features"))
            .expect("failed to preprocess .feature files into tempdir");

    futures::executor::block_on(
        TuiWorld::cucumber()
            .max_concurrent_scenarios(25)
            .fail_on_skipped()
            .filter_run_and_exit(stripped_features_path.clone(), move |feat, _, sc| {
                if sc.tags.iter().any(|t| t == "pending") {
                    return false;
                }
                if let Some(ref tag) = tag_filter {
                    let matches_feature = feat.tags.iter().any(|t| t == tag.as_str());
                    let matches_scenario = sc.tags.iter().any(|t| t == tag.as_str());
                    if !matches_feature && !matches_scenario {
                        return false;
                    }
                }
                if let Some((idx, total)) = shard {
                    if let Some(plan) = shard_plan.as_ref() {
                        let key = (feat.name.clone(), sc.name.clone());
                        if let Some(assigned) = plan.get(&key) {
                            if *assigned != idx {
                                return false;
                            }
                        } else if stable_hash(&feat.name, &sc.name) % total != idx {
                            return false;
                        }
                    } else if stable_hash(&feat.name, &sc.name) % total != idx {
                        return false;
                    }
                }
                tag_filter.is_some()
                    || feat.tags.iter().any(|t| t == "done")
                    || sc.tags.iter().any(|t| t == "done")
            }),
    );
}
