//! A follower's own tokens (epic #2705 B11 review 2): kept unless asked to
//! replace them; a set taken from a primary, or none, needs no asking.
use std::sync::Mutex;

use super::*;
use crate::application::project_board::ports::TokenError;
use crate::application::project_board::ports::replication::FollowedTokens;

struct Held(Mutex<(FollowedTokens, usize)>);

impl Held {
    fn of(held: usize, copied: bool) -> Arc<Self> {
        Arc::new(Self(Mutex::new((FollowedTokens { held, copied }, 0))))
    }

    fn set_asides(&self) -> usize {
        self.0.lock().unwrap().1
    }
}

impl FollowTokens for Held {
    fn followed(&self) -> Result<FollowedTokens, TokenError> {
        Ok(self.0.lock().unwrap().0)
    }

    fn set_aside(&self) -> Result<(), TokenError> {
        let mut held = self.0.lock().unwrap();
        *held = (
            FollowedTokens {
                held: 0,
                copied: true,
            },
            held.1 + 1,
        );
        Ok(())
    }
}

#[test]
fn tokens_of_its_own_are_refused_unless_asked_to_replace_them() {
    let held = Held::of(2, false);
    let ready = ReadyFollowerTokens::new(Arc::clone(&held) as _);
    let refused = ready.ready(false).unwrap_err();
    assert!(refused.contains("2 token(s) of its own"), "{refused}");
    assert!(refused.contains("--replace-tokens"), "{refused}");
    assert_eq!(held.set_asides(), 0, "a refusal sets nothing aside");
    assert_eq!(ready.ready(true), Ok(()));
    assert_eq!(held.set_asides(), 1);
    assert_eq!(held.followed().unwrap().held, 0, "refused at once");
}

#[test]
fn a_primarys_copied_set_or_none_needs_no_asking() {
    for (held, copied) in [(3, true), (0, false), (0, true)] {
        let tokens = Held::of(held, copied);
        let ready = ReadyFollowerTokens::new(Arc::clone(&tokens) as _);
        assert_eq!(ready.ready(false), Ok(()), "{held} {copied}");
        assert_eq!(ready.ready(true), Ok(()), "{held} {copied}");
        assert_eq!(tokens.set_asides(), 0, "a primary's set is never set aside");
    }
}
