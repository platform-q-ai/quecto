//! A follower's own tokens (epic #2705 B11 review 2): refused; a set taken
//! from a primary, or none, is a follower's.
use super::*;
use crate::application::project_board::ports::TokenError;
use crate::application::project_board::ports::replication::FollowedTokens;

struct Held(FollowedTokens);

impl FollowTokens for Held {
    fn followed(&self) -> Result<FollowedTokens, TokenError> {
        Ok(self.0)
    }
}

fn ready(held: usize, copied: bool) -> Result<(), String> {
    ReadyFollowerTokens::new(Arc::new(Held(FollowedTokens { held, copied }))).ready()
}

#[test]
fn tokens_of_its_own_are_refused() {
    let refused = ready(2, false).unwrap_err();
    assert!(refused.contains("2 token(s) of its own"), "{refused}");
    assert!(refused.contains("--state <dir>"), "{refused}");
}

#[test]
fn a_primarys_copied_set_or_none_is_a_followers() {
    for (held, copied) in [(3, true), (0, false), (0, true)] {
        assert_eq!(ready(held, copied), Ok(()), "{held} {copied}");
    }
}
