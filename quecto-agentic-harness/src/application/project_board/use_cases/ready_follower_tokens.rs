//! A follower's tokens as it starts (epic #2705 B11): a follower holds its
//! primary's tokens. A server holding tokens of its own (any set it did not
//! take from a primary: before it first follows, or issued after a
//! promotion) is refused, unless asked to replace them; then they are set
//! aside and refused at once, before its primary's are copied.
use std::sync::Arc;

use crate::application::project_board::ports::replication::FollowTokens;

/// The policy for a follower's own tokens.
pub struct ReadyFollowerTokens {
    tokens: Arc<dyn FollowTokens>,
}

impl ReadyFollowerTokens {
    pub fn new(tokens: Arc<dyn FollowTokens>) -> Self {
        Self { tokens }
    }

    /// Readies the tokens for following; `replace` says the server's own
    /// may be replaced (`--replace-tokens`).
    ///
    /// # Errors
    /// Tokens of its own and no `replace` (what to do is said), or the
    /// store failed.
    pub fn ready(&self, replace: bool) -> Result<(), String> {
        let followed = self.tokens.followed().map_err(|error| error.to_string())?;
        // None held, or the set its primary gave: a follower's already.
        let followers = followed.held == 0 || followed.copied;
        match (followers, replace) {
            (true, _) => Ok(()),
            (false, true) => self.tokens.set_aside().map_err(|error| error.to_string()),
            (false, false) => Err(format!(
                "this server holds {} token(s) of its own: a follower holds its primary's \
                 instead. Give --replace-tokens to replace them (they are kept as \
                 tokens.json.pre-follow, or -1, -2 and on beside an earlier one, and refused \
                 at once), or give the follower its own --state <dir>",
                followed.held
            )),
        }
    }
}

#[cfg(test)]
#[path = "ready_follower_tokens_tests.rs"]
mod tests;
