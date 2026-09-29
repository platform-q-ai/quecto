//! `IdSource` on `Uuid4Ids` (#2270): Python's `uuid.uuid4().hex`.
use quecto::application::swarm::ports::IdSource;
use quecto::infrastructure::persistence::swarm_board::ids::Uuid4Ids;

#[test]
fn ids_are_fresh_32_lowercase_hex_digits() {
    let ids: &dyn IdSource = &Uuid4Ids;
    let first = ids.hex32();
    let second = ids.hex32();
    for id in [&first, &second] {
        assert_eq!(id.len(), 32, "{id}");
        assert!(
            id.bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "{id}"
        );
    }
    assert_ne!(first, second, "two draws differ");
}

/// A version 4, RFC 4122 variant UUID, as `uuid.uuid4()` draws: in the
/// hyphenated form the version nibble is index 14 and the variant nibble
/// index 19; without hyphens they are indices 12 and 16.
#[test]
fn ids_are_version_4_rfc_4122_uuids() {
    let ids: &dyn IdSource = &Uuid4Ids;
    for _ in 0..64 {
        let id = ids.hex32();
        let hyphenated = format!(
            "{}-{}-{}-{}-{}",
            &id[..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..]
        );
        assert_eq!(hyphenated.as_bytes()[14], b'4', "version: {hyphenated}");
        assert!(
            matches!(hyphenated.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "variant: {hyphenated}"
        );
    }
}
