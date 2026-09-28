use super::Uuid4Ids;
use crate::application::swarm::ports::IdSource;

#[test]
fn ids_are_32_lowercase_hex_digits_of_a_version_4_uuid() {
    let id = Uuid4Ids.hex32();
    assert_eq!(id.len(), 32);
    assert!(
        id.bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
        "{id}"
    );
    // The version nibble of `uuid4().hex`.
    assert_eq!(&id[12..13], "4", "{id}");
    assert_ne!(Uuid4Ids.hex32(), id);
}
