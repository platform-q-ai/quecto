//! Search adapters (#2136 slice B): relevance judging through TypeSafe's
//! Jev, and the local JSONL search log.

pub mod search_log;
pub mod typesafe_judge;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
        match typesafe_key() {
            Some(key) => {
                let judge = TypeSafeJudge::new(
                    TYPESAFE_ENDPOINT,
                    key,
                    &relevance.model,
                    Duration::from_secs(relevance.timeout_secs.max(1)),
                );
                tool = tool.with_relevance(Arc::new(judge), relevance.max_candidates);
            }
            None => tracing::warn!(
                "tools.grep.relevance is enabled but no TypeSafe key was found (TYPESAFE_API_KEY or ~/.config/typesafe/api_key): rank_by is not offered"
            ),
        }
    }
    if wiring.config.log.enabled {
        tool = tool.with_search_log(Arc::new(JsonlSearchLog::new(
            Path::new(&wiring.base_dir),
            &wiring.session_key,
        )));
    }
    Arc::new(tool)
}
