//! The host devices a container config may hand its container.

/// The host devices `quecto container init --device` may bake into the
/// standard entry (the official adapter keeps the same allowlist): KVM
/// alone, so a swarm can run a headless Android emulator. Matched exactly.
pub const ALLOWED_CONTAINER_DEVICES: &[&str] = &["/dev/kvm"];

/// Whether `device` is on [`ALLOWED_CONTAINER_DEVICES`], spelled exactly.
pub fn is_allowed_container_device(device: &str) -> bool {
    ALLOWED_CONTAINER_DEVICES.contains(&device)
}
