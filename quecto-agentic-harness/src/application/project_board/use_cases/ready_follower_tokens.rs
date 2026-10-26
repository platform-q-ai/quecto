//! A follower's tokens as it starts (epic #2705 B11): a follower holds its
//! primary's tokens. A server holding tokens of its own (any set it did not
//! take from a primary: before it first follows, or issued after a
//! promotion) is refused, so no token of its own stays live on a follower.
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

    /// Readies the tokens for following.
    ///
    /// # Errors
    /// Tokens of its own (what to do is said), or the store failed.
    pub fn ready(&self) -> Result<(), String> {
        let followed = self.tokens.followed().map_err(|error| error.to_string())?;
        // None held, or the set its primary gave: a follower's already.
        match followed.held == 0 || followed.copied {
            true => Ok(()),
            false => Err(format!(
                "this server holds {} token(s) of its own: a follower holds its primary's \
                 instead. Revoke them first, or give the follower its own --state <dir>",
                followed.held
            )),
        }
    }
}

#[cfg(test)]
#[path = "ready_follower_tokens_tests.rs"]
mod tests;
