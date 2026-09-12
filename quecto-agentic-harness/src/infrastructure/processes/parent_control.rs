//! Out-of-band delivery and presentation of the parent-control capability
//! (#1935).
//!
//! The launcher mints a [`ParentControlCredential`] per child, writes it to a
//! private (`0600`, `O_EXCL`) sidecar under the runtime directory, and passes
//! only the sidecar *path* on the child's argv. The child reads and removes
//! the sidecar before it announces its socket, so the material never lingers
//! on disk, never appears in argv or the environment, and is never forwarded
//! to the child's own children (they get their own credentials from their
//! own launcher). The parent presents the credential once, as the first
//! frame on the connection it wants bound (the monitor connection); the
//! child-side schema lives in `interface::uds::parent_control::wire`.
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::domain::parent_control::{ParentControlCapability, ParentControlCredential};
use crate::domain::subagent_teardown::LaunchGeneration;

const SIDECAR_FORMAT: u32 = 1;

/// Wire `type` of the presentation frame the parent sends first.
pub const BIND_PARENT_CONTROL: &str = "bind_parent_control";

/// Process-wide launch generation: strictly increasing per launcher process,
/// so a re-used child uuid can never be bound by a stale credential.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

pub fn next_launch_generation() -> LaunchGeneration {
    LaunchGeneration::new(NEXT_GENERATION.fetch_add(1, Ordering::SeqCst))
}

/// Mint a fresh credential from OS randomness for the next generation.
pub fn mint_credential() -> ParentControlCredential {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    ParentControlCredential {
        generation: next_launch_generation(),
        capability: ParentControlCapability::from_random_bytes(&bytes),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SidecarWire {
    format: u32,
    generation: u64,
    capability: String,
}

/// Write the credential to `path` privately: owner-only, created
/// exclusively so a pre-planted file or symlink is refused.
pub fn write_sidecar(path: &Path, credential: &ParentControlCredential) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let wire = SidecarWire {
        format: SIDECAR_FORMAT,
        generation: credential.generation.get(),
        capability: credential.capability.expose().to_owned(),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&serde_json::to_vec(&wire).expect("sidecar is serializable"))?;
    file.sync_all()
}

/// Read the credential and remove the sidecar whatever the outcome: the
/// material is single-use and must not survive the child's startup.
pub fn take_sidecar(path: &Path) -> Result<ParentControlCredential, String> {
    let outcome = read_sidecar(path);
    let _ = std::fs::remove_file(path);
    outcome
}

fn read_sidecar(path: &Path) -> Result<ParentControlCredential, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("parent control sidecar {} unreadable: {e}", path.display()))?;
    let wire: SidecarWire = serde_json::from_slice(&bytes)
        .map_err(|e| format!("parent control sidecar {} malformed: {e}", path.display()))?;
    if wire.format != SIDECAR_FORMAT {
        return Err(format!(
            "parent control sidecar format {} unsupported (expected {SIDECAR_FORMAT})",
            wire.format
        ));
    }
    let capability = ParentControlCapability::parse(&wire.capability)
        .map_err(|e| format!("parent control sidecar {}: {e}", path.display()))?;
    Ok(ParentControlCredential {
        generation: LaunchGeneration::new(wire.generation),
        capability,
    })
}

/// The presentation the parent writes first on the connection it binds,
/// as one JSON line (no trailing newline; the caller frames it). The child
/// decodes it with `interface::uds::parent_control::wire`; the two sides are
/// pinned to one schema by that module's tests.
pub fn presentation_json(credential: &ParentControlCredential) -> String {
    serde_json::json!({
        "type": BIND_PARENT_CONTROL,
        "generation": credential.generation.get(),
        "capability": credential.capability.expose(),
    })
    .to_string()
}

#[cfg(test)]
#[path = "parent_control_tests.rs"]
mod tests;
