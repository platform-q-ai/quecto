//! Contract for the `FreshSessionIdentityGenerator` port (#1862, D7
//! #1976): every identity is a current-format, non-ephemeral user-chat
//! key (`chat-<secs>-<uniq>`), and two identities drawn in one process —
//! even within the same second — never collide.
use std::collections::BTreeSet;
use std::sync::Arc;

use quecto::application::sessions::ports::FreshSessionIdentityGenerator;
use quecto::domain::session::USER_CHAT_PREFIX;
use quecto::infrastructure::persistence::fresh_session_identity::ProcessClockIdentityGenerator;

fn under_test() -> Arc<dyn FreshSessionIdentityGenerator> {
    Arc::new(ProcessClockIdentityGenerator::new())
}

#[test]
fn identities_are_current_format_user_chat_keys() {
    let identity = under_test().fresh_identity();
    let key = identity.runtime_key();
    assert!(key.starts_with(USER_CHAT_PREFIX), "{key}");
    assert!(!identity.is_ephemeral());
    assert_eq!(identity.persisted_key(), Some(key));
    let mut parts = key.trim_start_matches(USER_CHAT_PREFIX).splitn(2, '-');
    let secs: u64 = parts.next().unwrap().parse().expect("wall-clock seconds");
    assert!(secs > 1_600_000_000, "a current wall clock: {secs}");
    let uniq = parts.next().expect("uniqueness token");
    assert!(u64::from_str_radix(uniq, 16).is_ok(), "hex token: {uniq}");
}

#[test]
fn identities_drawn_in_one_process_never_repeat() {
    let generator = under_test();
    let keys: BTreeSet<String> = (0..64)
        .map(|_| generator.fresh_identity().runtime_key().to_string())
        .collect();
    assert_eq!(keys.len(), 64, "same-second identities are distinct");
    // A second generator instance shares the process counter: still no
    // collision across instances.
    let other = under_test().fresh_identity();
    assert!(!keys.contains(other.runtime_key()));
}
