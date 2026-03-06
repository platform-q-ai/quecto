//! Hot-reload watcher: polls extension directories for changes and
//! triggers reload when file fingerprints change.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tokio::task::JoinHandle;

use super::registry::ExtensionRegistry;
use std::sync::{Arc, Mutex};

/// Compute a fingerprint (mtime + size) of all `extension.toml` files
/// in the given directories.
pub fn fingerprint_dirs(dirs: &[PathBuf]) -> HashMap<PathBuf, (SystemTime, u64)> {
    let mut map = HashMap::new();
    for dir in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest = path.join("extension.toml");
            if let Ok(meta) = std::fs::metadata(&manifest) {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let size = meta.len();
                map.insert(manifest, (mtime, size));
            }
        }
    }
    map
}

/// Spawn a background task that polls watch directories for changes
/// and reloads script extensions when detected.
///
/// Returns a `JoinHandle` that can be used to cancel the watcher.
pub fn spawn_watcher(
    registry: Arc<Mutex<ExtensionRegistry>>,
    poll_interval: Duration,
) -> JoinHandle<()> {
    let watch_dirs: Vec<PathBuf> = {
        let reg = registry.lock().unwrap();
        reg.watch_dirs().to_vec()
    };
    spawn_watcher_with_dirs(registry, poll_interval, watch_dirs)
}

/// Spawn watcher with explicit directories.
pub fn spawn_watcher_with_dirs(
    registry: Arc<Mutex<ExtensionRegistry>>,
    poll_interval: Duration,
    watch_dirs: Vec<PathBuf>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_fingerprint = fingerprint_dirs(&watch_dirs);
        loop {
            tokio::time::sleep(poll_interval).await;
            let current = fingerprint_dirs(&watch_dirs);
            if current != last_fingerprint {
                let mut reg = registry.lock().unwrap();
                reg.reload_scripts();
                let count = reg.extension_count();
                tracing::info!("extensions reloaded ({} total)", count);
                last_fingerprint = current;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_fingerprint_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let fp = fingerprint_dirs(&[tmp.path().to_path_buf()]);
        assert!(fp.is_empty());
    }

    #[test]
    fn test_fingerprint_nonexistent_dir() {
        let fp = fingerprint_dirs(&[PathBuf::from("/nonexistent/12345")]);
        assert!(fp.is_empty());
    }

    #[test]
    fn test_fingerprint_detects_new_file() {
        let tmp = TempDir::new().unwrap();
        let fp1 = fingerprint_dirs(&[tmp.path().to_path_buf()]);

        // Add an extension
        let ext_dir = tmp.path().join("test-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("extension.toml"), "name = \"test\"").unwrap();

        let fp2 = fingerprint_dirs(&[tmp.path().to_path_buf()]);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_fingerprint_stable() {
        let tmp = TempDir::new().unwrap();
        let ext_dir = tmp.path().join("test-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("extension.toml"), "name = \"test\"").unwrap();

        let fp1 = fingerprint_dirs(&[tmp.path().to_path_buf()]);
        let fp2 = fingerprint_dirs(&[tmp.path().to_path_buf()]);
        assert_eq!(fp1, fp2);
    }
}
