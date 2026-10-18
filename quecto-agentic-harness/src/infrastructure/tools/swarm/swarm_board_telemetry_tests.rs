//! #2303 round-3 review M1, round-4 review L1/L2/N4: a member's
//! `actor_ref` is redacted once and kept, for a bounded number of members,
//! only after the board accepted the member's call and only for an id
//! within the board's member-id bound, and is always the redacted, bounded
//! ref.
use super::{ACTOR_REF_CACHE, ACTOR_REF_CHARS, ActorRefs, Caller, actor_ref};
use crate::domain::swarm::validation::MEMBER_ID_MAX_BYTES;

#[test]
fn a_members_ref_is_the_redacted_bounded_ref_every_time() {
    let refs = ActorRefs::default();
    let secret = "sk-livedeadbeef0001abcdefghijklmnop";
    let long = "w".repeat(ACTOR_REF_CHARS + 40);
    for member in ["parent", secret, long.as_str()] {
        for caller in [Caller::Member, Caller::Unproven] {
            let first = refs.of(member, caller);
            assert_eq!(first, actor_ref(member));
            assert_eq!(refs.of(member, caller), first, "the same ref each time");
        }
    }
    assert!(!refs.of(secret, Caller::Member).contains("deadbeef"));
    assert_eq!(
        refs.of(&long, Caller::Member).chars().count(),
        ACTOR_REF_CHARS
    );
}

#[test]
fn only_a_bounded_number_of_members_is_kept() {
    let refs = ActorRefs::default();
    for index in 0..ACTOR_REF_CACHE * 3 {
        let member = format!("member-{index}");
        assert_eq!(refs.of(&member, Caller::Member), actor_ref(&member));
    }
    assert_eq!(refs.len(), ACTOR_REF_CACHE);
    // A member kept before the bound was reached is still answered.
    assert_eq!(refs.of("member-0", Caller::Member), actor_ref("member-0"));
    assert_eq!(refs.len(), ACTOR_REF_CACHE);
}

/// Round-4 review L1: a call the board did not accept from a member (a
/// refusal, or a membership-free method) names any id it likes; none of
/// them takes a place a member could use.
#[test]
fn refused_and_garbage_ids_are_never_kept() {
    let refs = ActorRefs::default();
    for index in 0..ACTOR_REF_CACHE * 2 {
        let garbage = format!("ghost-{index}\u{0}\u{202e}");
        assert_eq!(refs.of(&garbage, Caller::Unproven), actor_ref(&garbage));
    }
    assert_eq!(refs.len(), 0, "an unproven caller is redacted afresh");
    assert_eq!(refs.of("parent", Caller::Member), actor_ref("parent"));
    assert_eq!(refs.len(), 1, "a member still has its place");
}

/// Round-4 review L2: an id past the board's member-id bound is no
/// member's, whatever the call's outcome says, so it is redacted afresh
/// and never stored; an id at the bound is.
#[test]
fn an_over_long_id_is_never_stored() {
    let refs = ActorRefs::default();
    let over = "m".repeat(MEMBER_ID_MAX_BYTES + 1);
    assert_eq!(refs.of(&over, Caller::Member), actor_ref(&over));
    // Multi-byte characters: within the character cut, over the byte bound.
    let wide = "é".repeat(MEMBER_ID_MAX_BYTES / 2 + 1);
    assert_eq!(refs.of(&wide, Caller::Member), actor_ref(&wide));
    assert_eq!(refs.len(), 0, "an over-long id is not stored");
    let at_bound = "m".repeat(MEMBER_ID_MAX_BYTES);
    assert_eq!(refs.of(&at_bound, Caller::Member), actor_ref(&at_bound));
    assert_eq!(refs.len(), 1, "an id at the bound is stored");
}
