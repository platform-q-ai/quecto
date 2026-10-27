//! The host devices a container config may hand its container.

/// The host devices `quecto container init --device` may bake into the
/// standard entry (the official adapter keeps the same allowlist): KVM
/// alone, so a swarm can run a headless Android emulator. Matched exactly.
pub const ALLOWED_CONTAINER_DEVICES: &[&str] = &["/dev/kvm"];

/// Whether `device` is on [`ALLOWED_CONTAINER_DEVICES`], spelled exactly.
pub fn is_allowed_container_device(device: &str) -> bool {
    ALLOWED_CONTAINER_DEVICES.contains(&device)
}

/// Where a device init was about to bake in came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSource {
    /// `--device` on the command line.
    Flag,
    /// The existing standard entry's own create argv (a re-init without
    /// `--device` keeps it).
    ExistingEntry,
}

/// A device off [`ALLOWED_CONTAINER_DEVICES`]: which one, where it came
/// from, and (in its display) the way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRefusal {
    pub device: String,
    pub source: DeviceSource,
}

impl std::fmt::Display for DeviceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let allowed = ALLOWED_CONTAINER_DEVICES.join(", ");
        let device = &self.device;
        match self.source {
            DeviceSource::Flag => write!(
                f,
                "--device: only {allowed} is supported, not {device:?} (given with --device); pass --device {allowed}, or --no-device for none"
            ),
            DeviceSource::ExistingEntry => write!(
                f,
                "--device: only {allowed} is supported, not {device:?} (kept from the existing standard entry); pass --device {allowed} to replace the entry's list, or --no-device to remove it"
            ),
        }
    }
}
