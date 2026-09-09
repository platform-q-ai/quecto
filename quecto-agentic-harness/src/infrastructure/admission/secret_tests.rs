use super::*;

#[test]
fn mint_yields_sixty_four_hex_characters() {
    let secret = RandomSecretSource.mint();
    assert_eq!(secret.len(), 64);
    assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()));
}
