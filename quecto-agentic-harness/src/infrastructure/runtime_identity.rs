//! Identifies the running executable independently of the workload checkout.
use crate::domain::request_observation::RuntimeIdentity;
use sha2::{Digest, Sha256};
use std::io::Read;

pub fn current() -> RuntimeIdentity {
    static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    static DIGEST: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    if STARTED.set(()).is_ok() {
        std::thread::spawn(|| {
            let _ = DIGEST.set(executable_digest().ok());
        });
    }
    RuntimeIdentity {
        pid: std::process::id(),
        process_instance_id: INSTANCE
            .get_or_init(|| uuid::Uuid::new_v4().to_string())
            .clone(),
        package_version: env!("CARGO_PKG_VERSION").into(),
        build_source_revision: option_env!("QUECTO_BUILD_SOURCE_REVISION").map(str::to_owned),
        build_dirty: match option_env!("QUECTO_BUILD_DIRTY") {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        },
        executable_digest_pending: DIGEST.get().is_none(),
        executable_sha256: DIGEST.get().cloned().flatten(),
    }
}
fn executable_digest() -> std::io::Result<String> {
    executable_digest_for(std::process::id())
}
fn executable_digest_for(pid: u32) -> std::io::Result<String> {
    #[cfg(target_os = "linux")]
    let executable = std::path::PathBuf::from(format!("/proc/{pid}/exe"));
    #[cfg(not(target_os = "linux"))]
    let executable = {
        let _ = pid;
        std::env::current_exe()?
    };
    let mut file = std::fs::File::open(executable)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
#[path = "runtime_identity_tests.rs"]
mod tests;
