//! #2303 round-3 review M1: a member's `actor_ref` is redacted once and
//! kept, for a bounded number of members, and is always the redacted,
//! bounded ref.
use super::{ACTOR_REF_CACHE, ACTOR_REF_CHARS, ActorRefs, actor_ref};

#[test]
fn a_members_ref_is_the_redacted_bounded_ref_every_time() {
    let refs = ActorRefs::default();
    let secret = "sk-livedeadbeef0001abcdefghijklmnop";
    let long = "w".repeat(ACTOR_REF_CHARS + 40);
    for member in ["parent", secret, long.as_str()] {
        let first = refs.of(member);
        assert_eq!(first, actor_ref(member));
        assert_eq!(refs.of(member), first, "the kept ref is the same");
    }
    assert!(!refs.of(secret).contains("deadbeef"));
    assert_eq!(refs.of(&long).chars().count(), ACTOR_REF_CHARS);
}

#[test]
fn only_a_bounded_number_of_members_is_kept() {
    let refs = ActorRefs::default();
    for index in 0..ACTOR_REF_CACHE * 3 {
        let member = format!("member-{index}");
        assert_eq!(refs.of(&member), actor_ref(&member));
    }
    assert_eq!(refs.len(), ACTOR_REF_CACHE);
    // A member kept before the bound was reached is still answered.
    assert_eq!(refs.of("member-0"), actor_ref("member-0"));
    assert_eq!(refs.len(), ACTOR_REF_CACHE);
}
