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

/// Type alias for a reload notification callback.
///
/// Called after `reload_scripts()` succeeds.  The `usize` argument is
/// the new extension count.  Used by the UDS layer to sync the tool
/// registry and broadcast `extensions_changed` events.
pub type ReloadCallback = Arc<dyn Fn(usize) + Send + Sync>;

/// Spawn a watcher that calls `on_reload` after each reload.
///
/// This is the primary entry point for the UDS agent: it triggers
/// tool registry sync and event broadcast via the callback.
pub fn spawn_watcher_with_callback(
    registry: Arc<Mutex<ExtensionRegistry>>,
    poll_interval: Duration,
    on_reload: ReloadCallback,
) -> JoinHandle<()> {
    let watch_dirs: Vec<PathBuf> = {
        let reg = registry.lock().unwrap();
        reg.watch_dirs().to_vec()
    };
    tokio::spawn(async move {
        let mut last_fingerprint = fingerprint_dirs(&watch_dirs);
        loop {
            tokio::time::sleep(poll_interval).await;
            let current = fingerprint_dirs(&watch_dirs);
            if current != last_fingerprint {
                let count = {
                    let mut reg = registry.lock().unwrap();
                    reg.reload_scripts();
                    reg.extension_count()
                };
                tracing::info!("extensions reloaded ({} total)", count);
                on_reload(count);
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

    #[tokio::test]
    async fn test_spawn_watcher_with_callback_fires_on_change() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let mut reg = ExtensionRegistry::new();
        reg.set_watch_dirs(vec![dir.clone()]);
        let registry = Arc::new(Mutex::new(reg));

        let callback_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = callback_count.clone();
        let cb: ReloadCallback = Arc::new(move |_count| {
            cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        // Add an extension to trigger reload BEFORE starting the watcher,
        // so the first fingerprint includes nothing, then the change is detected.
        let ext_dir = dir.join("test-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        let manifest = r#"
name = "test"
description = "Test"
parameters_schema = '{"type":"object"}'
command = "./run.sh"
"#;

        let handle = spawn_watcher_with_callback(registry, Duration::from_millis(50), cb);

        // Give watcher time to take initial fingerprint (empty), then add extension
        tokio::time::sleep(Duration::from_millis(80)).await;

        std::fs::write(ext_dir.join("extension.toml"), manifest).unwrap();
        let script = "#!/bin/sh\necho '{\"content\":\"ok\",\"is_error\":false}'";
        std::fs::write(ext_dir.join("run.sh"), script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                ext_dir.join("run.sh"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }

        // Wait for at least two poll cycles after file creation
        tokio::time::sleep(Duration::from_millis(300)).await;
        handle.abort();

        assert!(
            callback_count.load(std::sync::atomic::Ordering::SeqCst) >= 1,
            "callback should have been called at least once"
        );
    }

    #[tokio::test]
    async fn test_spawn_watcher_with_callback_no_spurious_calls() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let mut reg = ExtensionRegistry::new();
        reg.set_watch_dirs(vec![dir.clone()]);
        let registry = Arc::new(Mutex::new(reg));

        let callback_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = callback_count.clone();
        let cb: ReloadCallback = Arc::new(move |_count| {
            cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        let handle = spawn_watcher_with_callback(registry, Duration::from_millis(50), cb);

        // Don't change anything — callback should not fire
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();

        assert_eq!(
            callback_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "callback should not fire when no changes occur"
        );
    }
}
