// Session reload use case: strip stale tool history and clear spill index.

use crate::domain::session::{ContextSpillStore, Session, SessionStore, strip_tool_history};

/// Execute the /reload command for a given session key.
///
/// 1. Loads the session by `session_key` from the session store.
/// 2. Applies `strip_tool_history()` to remove stale tool evidence.
/// 3. Saves the filtered session.
/// 4. Atomically clears `spill.jsonl` for this session key.
/// 5. Returns a human-readable summary safe to send to the user.
///
/// The caller is responsible for building the session key so that the key
/// is derived in a single place.
///
/// This is application-layer orchestration: it coordinates domain logic
/// (`strip_tool_history`) and infrastructure ports (`SessionStore`, `ContextSpillStore`)
/// with no direct I/O of its own.
pub async fn execute_reload(
    session_key: &str,
    session_store: &dyn SessionStore,
    spill_store: &dyn ContextSpillStore,
) -> String {
    // O(n) in message count — acceptable for a user-triggered command (not a hot path).
    let session = match session_store.load(session_key).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return "Session reloaded. No existing session found — nothing to clean.".to_string();
        }
        Err(e) => {
            tracing::error!(error = %e, key = session_key, "failed to load session for /reload");
            return "Error: could not load session — please try again.".to_string();
        }
    };

    let original_count = session.messages.len();
    let filtered = strip_tool_history(&session.messages);
    let filtered_count = filtered.len();
    let removed = original_count.saturating_sub(filtered_count);

    let new_session = Session {
        key: session_key.to_string(),
        messages: filtered,
    };

    if let Err(e) = session_store.save(&new_session).await {
        tracing::error!(error = %e, key = session_key, "failed to save session after /reload");
        return "Error: could not save session — please try again.".to_string();
    }

    if let Err(e) = spill_store.clear(session_key).await {
        tracing::warn!(error = %e, key = session_key, "failed to clear spill on /reload");
        return format!(
            "Session reloaded. Kept {} messages, removed {} tool calls. \
             Warning: recall history could not be cleared.",
            filtered_count, removed
        );
    }

    format!(
        "Session reloaded. Kept {} messages, removed {} tool calls. Recall history cleared.",
        filtered_count, removed
    )
}
