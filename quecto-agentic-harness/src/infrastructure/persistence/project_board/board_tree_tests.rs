use super::*;

/// Every byte a kept name may hold shows as itself, so a name quecto
/// keeps is listed exactly as it is named.
#[test]
fn every_byte_a_kept_name_holds_shows_as_itself() {
    let allowed = (b'0'..=b'9')
        .chain(b'a'..=b'z')
        .chain(b'A'..=b'Z')
        .chain(NAME_PUNCTUATION.bytes());
    for byte in allowed {
        assert!(shown_char(char::from(byte)), "{:?}", char::from(byte));
        assert_eq!(shown_name(&[byte]), char::from(byte).to_string());
    }
}
