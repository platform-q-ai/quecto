//! Search adapters (#2136 slice B): relevance judging through TypeSafe's
//! Jev, and the local JSONL search log.

pub mod search_log;
pub mod typesafe_judge;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::tools::ports::Tool;
use crate::infrastructure::config::GrepToolConfig;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::grep::GrepTool;

use search_log::JsonlSearchLog;
use typesafe_judge::{TYPESAFE_ENDPOINT, TypeSafeJudge, typesafe_key};

/// What a session's grep tool is wired with (#2136): its settings, and
/// where and for whom the search log is written.
#[derive(Debug, Clone)]
pub struct GrepWiring {
    pub config: GrepToolConfig,
    pub base_dir: PathBuf,
    pub session_key: String,
}

/// The grep tool with relevance ranking through TypeSafe's Jev when
/// `tools.grep.relevance.enabled` and a TypeSafe key is found, and the
/// local search log when `tools.grep.log.enabled` (the default).
pub fn build_grep_tool(
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    wiring: &GrepWiring,
) -> Arc<dyn Tool> {
    let mut tool = GrepTool::new(workspace, sandbox);
    let relevance = &wiring.config.relevance;
    if relevance.enabled {
        match ranking_judge(relevance) {
            Ok((judge, max_candidates)) => {
                tool = tool.with_relevance(Arc::new(judge), max_candidates);
            }
            Err(reason) => tracing::warn!(%reason, "grep ranking is not offered"),
        }
    }
    if wiring.config.log.enabled {
        let log = JsonlSearchLog::new(Path::new(&wiring.base_dir), &wiring.session_key);
        // Results from the log are never shown: its lines hold past
        // patterns. A relative base dir is taken from the working directory.
        match std::path::absolute(log.dir()) {
            Ok(dir) => tool = tool.excluding(dir),
            Err(error) => {
                tracing::warn!(%error, "the search log's directory could not be resolved; its results are not filtered")
            }
        }
        tool = tool.with_search_log(Arc::new(log));
    }
    Arc::new(tool)
}

/// The judge for enabled ranking: settings in range, a TypeSafe key, and a
/// client; otherwise why ranking is not offered.
fn ranking_judge(
    relevance: &crate::infrastructure::config::grep_tool::GrepRelevanceConfig,
) -> Result<(TypeSafeJudge, usize), String> {
    let (max_candidates, timeout) = relevance.limits()?;
    let key = typesafe_key().ok_or(
        "no TypeSafe key was found (TYPESAFE_API_KEY or ~/.config/typesafe/api_key)".to_string(),
    )?;
    let judge = TypeSafeJudge::new(TYPESAFE_ENDPOINT, key, &relevance.model, timeout)?;
    Ok((judge, max_candidates))
}
