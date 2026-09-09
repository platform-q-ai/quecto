//! AdmissionSecretSource contract: capability material is opaque, non-empty and
//! never repeats within an authority.
use quecto::application::ports::AdmissionSecretSource;
use quecto::infrastructure::admission::RandomSecretSource;
use std::collections::BTreeSet;

#[test]
fn random_secrets_are_long_opaque_and_unique() {
    let mut source = RandomSecretSource;
    let secrets: BTreeSet<String> = (0..64).map(|_| source.mint()).collect();
    assert_eq!(secrets.len(), 64, "no repeats");
    for secret in &secrets {
        assert!(secret.len() >= 32, "at least 128 bits encoded");
        assert!(
            secret.chars().all(|c| c.is_ascii_alphanumeric()),
            "safe in JSON and files"
        );
    }
}
