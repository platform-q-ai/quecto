use super::*;

#[test]
fn identities_are_current_format_chat_keys_and_never_repeat_in_one_process() {
    let generator = ProcessClockIdentityGenerator::new();
    let a = generator.fresh_identity();
    let b = generator.fresh_identity();
    assert!(a.runtime_key().starts_with("chat-"));
    assert_ne!(a, b, "the per-process counter separates same-second keys");
    let parts: Vec<&str> = a.runtime_key().splitn(3, '-').collect();
    assert_eq!(parts.len(), 3);
    assert!(parts[1].parse::<u64>().is_ok(), "seconds: {}", parts[1]);
    assert!(
        u64::from_str_radix(parts[2], 16).is_ok(),
        "hex uniqueness token: {}",
        parts[2]
    );
    assert!(!a.is_ephemeral());
}
