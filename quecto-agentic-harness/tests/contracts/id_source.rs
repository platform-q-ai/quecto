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
