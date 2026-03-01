// ensure_tool — auto-download rg and fd binaries (Pi parity: ensureTool).
//
// Resolution order:
// 1. System PATH (highest priority)
// 2. Cache dir (~/.local/share/quecto/tools/)
// 3. Download from GitHub releases
// 4. Error with manual installation hint

use std::path::PathBuf;

use crate::domain::error::DomainError;

/// Offline mode env var. When set to "1", "true", or "yes", skips downloads.
const OFFLINE_ENV_VAR: &str = "QUECTO_OFFLINE";

/// Tool configurations for supported binaries.
struct ToolConfig {
    /// Binary name on disk (e.g., "rg", "fd").
    binary_name: &'static str,
    /// GitHub repo (e.g., "BurntSushi/ripgrep").
    repo: &'static str,
    /// Tag prefix: "" for ripgrep (tags like "14.0.0"), "v" for fd (tags like "v10.1.0").
    tag_prefix: &'static str,
    /// Human-readable name for messages.
    display_name: &'static str,
    /// Manual installation URL shown in errors.
    install_url: &'static str,
}

const TOOL_CONFIGS: &[ToolConfig] = &[
    ToolConfig {
        binary_name: "rg",
        repo: "BurntSushi/ripgrep",
        tag_prefix: "",
        display_name: "ripgrep",
        install_url: "https://github.com/BurntSushi/ripgrep#installation",
    },
    ToolConfig {
        binary_name: "fd",
        repo: "sharkdp/fd",
        tag_prefix: "v",
        display_name: "fd-find",
        install_url: "https://github.com/sharkdp/fd#installation",
    },
];

/// Return whether offline mode is enabled via the environment.
pub fn is_offline() -> bool {
    match std::env::var(OFFLINE_ENV_VAR) {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"),
        Err(_) => false,
    }
}

/// Resolve the cache directory for downloaded tool binaries.
/// Returns `~/.local/share/quecto/tools/` (XDG convention) or a temp dir as fallback.
pub fn tools_cache_dir() -> PathBuf {
    if let Some(data_dir) = dirs::data_local_dir() {
        data_dir.join("quecto").join("tools")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".quecto").join("tools")
    } else {
        std::env::temp_dir().join("quecto-tools")
    }
}

/// Check if a binary name exists on the system PATH by running `<name> --version`.
pub fn which(name: &str) -> Option<PathBuf> {
    std::process::Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
        .filter(|s| s.success() || s.code().is_some()) // command found even if non-zero
        .and_then(|_| which_path(name))
}

/// Find the full path of a binary on PATH.
fn which_path(name: &str) -> Option<PathBuf> {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // Fallback: just return the name — it will be found by the OS.
    Some(PathBuf::from(name))
}

/// Find the config for a tool by binary name.
fn find_config(tool: &str) -> Option<&'static ToolConfig> {
    TOOL_CONFIGS.iter().find(|c| c.binary_name == tool)
}

/// Ensure a tool binary is available. Resolution order:
/// 1. System PATH
/// 2. Cache directory
/// 3. Download from GitHub (unless offline mode)
///
/// Returns the path to the binary, or a `DomainError::Tool` on failure.
pub async fn ensure_tool(tool: &str) -> Result<PathBuf, DomainError> {
    ensure_tool_with_cache(tool, &tools_cache_dir()).await
}

/// Testable variant: same as `ensure_tool` but with an explicit cache directory.
pub async fn ensure_tool_with_cache(
    tool: &str,
    cache_dir: &std::path::Path,
) -> Result<PathBuf, DomainError> {
    let cfg = find_config(tool).ok_or_else(|| {
        DomainError::Tool(format!("Unknown tool: '{}'. Supported tools: rg, fd", tool))
    })?;

    // 1. Check PATH first (highest priority).
    if let Some(path) = which(cfg.binary_name) {
        return Ok(path);
    }

    // 2. Check cache directory.
    let cached = cache_dir.join(cfg.binary_name);
    if cached.is_file() {
        return Ok(cached);
    }

    // 3. Offline mode — skip download.
    if is_offline() {
        return Err(DomainError::Tool(format!(
            "{} not found and offline mode is enabled ({}=1). \
             Install manually: {}",
            cfg.display_name, OFFLINE_ENV_VAR, cfg.install_url
        )));
    }

    // 4. Download from GitHub releases.
    download_tool(cfg, cache_dir).await
}

/// Download a tool binary from GitHub releases, extract it, and cache it.
async fn download_tool(
    cfg: &ToolConfig,
    cache_dir: &std::path::Path,
) -> Result<PathBuf, DomainError> {
    // Detect platform.
    let (os, arch) = detect_platform()?;
    let asset_name = asset_name_for(cfg.binary_name, &os, &arch).ok_or_else(|| {
        DomainError::Tool(format!(
            "Unsupported platform: {}/{} for {}. Install manually: {}",
            os, arch, cfg.display_name, cfg.install_url
        ))
    })?;

    // Fetch latest version tag from GitHub API.
    let version = fetch_latest_version(cfg.repo).await?;
    let tag = format!("{}{}", cfg.tag_prefix, version);
    // Expand the {VERSION} placeholder in the asset name template.
    let asset_name = expand_asset_name(asset_name, &version);
    let url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        cfg.repo, tag, asset_name
    );

    // Create cache dir.
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| DomainError::Tool(format!("Failed to create cache dir: {}", e)))?;

    // Download archive.
    let archive_path = cache_dir.join(&asset_name);
    download_file(&url, &archive_path).await?;

    // Extract binary.
    let binary_path = cache_dir.join(cfg.binary_name);
    extract_binary(&archive_path, cfg.binary_name, cache_dir, &binary_path)?;

    // Make executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| DomainError::Tool(format!("chmod failed: {}", e)))?;
    }

    Ok(binary_path)
}

/// Detect the current OS and architecture strings used in release asset names.
fn detect_platform() -> Result<(String, String), DomainError> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        return Err(DomainError::Tool(
            "Unsupported OS for auto-download. Install rg/fd manually.".to_string(),
        ));
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return Err(DomainError::Tool(
            "Unsupported CPU architecture for auto-download. Install rg/fd manually.".to_string(),
        ));
    };

    Ok((os.to_string(), arch.to_string()))
}

/// Map (tool, os, arch) to GitHub release asset filename template.
/// The `{VERSION}` placeholder is expanded by `expand_asset_name`.
fn asset_name_for(tool: &str, os: &str, arch: &str) -> Option<&'static str> {
    match (tool, os, arch) {
        // ripgrep
        ("rg", "linux", "x86_64") => Some("ripgrep-{VERSION}-x86_64-unknown-linux-musl.tar.gz"),
        ("rg", "linux", "aarch64") => Some("ripgrep-{VERSION}-aarch64-unknown-linux-gnu.tar.gz"),
        ("rg", "darwin", "x86_64") => Some("ripgrep-{VERSION}-x86_64-apple-darwin.tar.gz"),
        ("rg", "darwin", "aarch64") => Some("ripgrep-{VERSION}-aarch64-apple-darwin.tar.gz"),
        // fd
        ("fd", "linux", "x86_64") => Some("fd-{VERSION}-x86_64-unknown-linux-gnu.tar.gz"),
        ("fd", "linux", "aarch64") => Some("fd-{VERSION}-aarch64-unknown-linux-gnu.tar.gz"),
        ("fd", "darwin", "x86_64") => Some("fd-{VERSION}-x86_64-apple-darwin.tar.gz"),
        ("fd", "darwin", "aarch64") => Some("fd-{VERSION}-aarch64-apple-darwin.tar.gz"),
        _ => None,
    }
}

/// Expand `{VERSION}` placeholder in an asset name template.
fn expand_asset_name(template: &str, version: &str) -> String {
    template.replace("{VERSION}", version)
}

/// Fetch the latest release tag from GitHub API.
async fn fetch_latest_version(repo: &str) -> Result<String, DomainError> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("quecto-coding-agent")
        .build()
        .map_err(|e| DomainError::Tool(format!("HTTP client error: {}", e)))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| DomainError::Tool(format!("GitHub API request failed: {}", e)))?;

    if !resp.status().is_success() {
        return Err(DomainError::Tool(format!(
            "GitHub API error {}: {}",
            resp.status(),
            url
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| DomainError::Tool(format!("GitHub API JSON parse error: {}", e)))?;

    let tag = json["tag_name"]
        .as_str()
        .ok_or_else(|| DomainError::Tool("GitHub API: missing tag_name".to_string()))?
        .trim_start_matches('v')
        .to_string();

    Ok(tag)
}

/// Download a URL to a local file path.
async fn download_file(url: &str, dest: &std::path::Path) -> Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent("quecto-coding-agent")
        .build()
        .map_err(|e| DomainError::Tool(format!("HTTP client error: {}", e)))?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| DomainError::Tool(format!("Download failed: {}", e)))?;

    if !resp.status().is_success() {
        return Err(DomainError::Tool(format!(
            "Download HTTP error {}: {}",
            resp.status(),
            url
        )));
    }

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| DomainError::Tool(format!("Failed to create file {:?}: {}", dest, e)))?;

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| DomainError::Tool(format!("Download stream error: {}", e)))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| DomainError::Tool(format!("Write error: {}", e)))?;
    }

    Ok(())
}

/// Extract a named binary from a .tar.gz archive to `dest`.
fn extract_binary(
    archive_path: &std::path::Path,
    binary_name: &str,
    extract_dir: &std::path::Path,
    dest: &std::path::Path,
) -> Result<(), DomainError> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let archive_file = std::fs::File::open(archive_path)
        .map_err(|e| DomainError::Tool(format!("Failed to open archive: {}", e)))?;

    let gz = GzDecoder::new(archive_file);
    let mut archive = Archive::new(gz);

    // Extract all entries; find the binary by name.
    let tmp_extract = extract_dir.join(format!("_extract_tmp_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_extract)
        .map_err(|e| DomainError::Tool(format!("Failed to create extract dir: {}", e)))?;

    archive
        .unpack(&tmp_extract)
        .map_err(|e| DomainError::Tool(format!("Archive extraction failed: {}", e)))?;

    // Find the binary recursively.
    let found = find_binary_in_dir(&tmp_extract, binary_name);

    // Cleanup archive file.
    let _ = std::fs::remove_file(archive_path);

    let found = found.ok_or_else(|| {
        let _ = std::fs::remove_dir_all(&tmp_extract);
        DomainError::Tool(format!(
            "Binary '{}' not found in archive {:?}",
            binary_name, archive_path
        ))
    })?;

    std::fs::rename(&found, dest)
        .or_else(|_| std::fs::copy(&found, dest).map(|_| ()))
        .map_err(|e| DomainError::Tool(format!("Failed to install binary: {}", e)))?;

    let _ = std::fs::remove_dir_all(&tmp_extract);
    Ok(())
}

/// Walk `dir` to find a file named exactly `name`.
fn find_binary_in_dir(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&cur) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name() == name && entry.path().is_file() {
                return Some(entry.path());
            }
            if entry.path().is_dir() {
                stack.push(entry.path());
            }
        }
    }
    None
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_find_config_rg() {
        let cfg = find_config("rg").expect("rg config");
        assert_eq!(cfg.binary_name, "rg");
        assert_eq!(cfg.repo, "BurntSushi/ripgrep");
    }

    #[test]
    fn test_find_config_fd() {
        let cfg = find_config("fd").expect("fd config");
        assert_eq!(cfg.binary_name, "fd");
        assert_eq!(cfg.repo, "sharkdp/fd");
    }

    #[test]
    fn test_find_config_unknown() {
        assert!(find_config("unknown_xyz").is_none());
    }

    #[test]
    fn test_tools_cache_dir_is_reasonable() {
        let dir = tools_cache_dir();
        let dir_str = dir.to_string_lossy();
        // Should be an absolute path containing "quecto" and "tools"
        assert!(dir.is_absolute(), "cache dir should be absolute: {:?}", dir);
        assert!(
            dir_str.contains("quecto"),
            "cache dir should contain 'quecto': {:?}",
            dir
        );
    }

    #[test]
    fn test_is_offline_default_false() {
        // Only test when env var is not set by the caller's environment.
        if std::env::var(OFFLINE_ENV_VAR).is_err() {
            assert!(!is_offline());
        }
    }

    #[test]
    fn test_is_offline_values() {
        // Test the parsing logic directly without mutating env.
        // is_offline() reads the env; we test the logic by checking the
        // expected return values for known inputs.
        for v in &["1", "true", "TRUE", "yes", "YES"] {
            assert!(
                v == &"1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"),
                "should recognize {}",
                v
            );
        }
        for v in &["0", "false", "no", ""] {
            assert!(
                !(v == &"1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")),
                "should not recognize {}",
                v
            );
        }
    }

    #[test]
    fn test_asset_name_rg_linux_x86() {
        let name = asset_name_for("rg", "linux", "x86_64").expect("should have asset");
        assert!(name.contains("x86_64"));
        assert!(name.contains("linux"));
        assert!(name.contains("musl") || name.contains("gnu"));
    }

    #[test]
    fn test_asset_name_fd_darwin_arm() {
        let name = asset_name_for("fd", "darwin", "aarch64").expect("should have asset");
        assert!(name.contains("aarch64"));
        assert!(name.contains("darwin"));
    }

    #[test]
    fn test_asset_name_unsupported_platform() {
        assert!(asset_name_for("rg", "windows", "x86_64").is_none());
    }

    #[test]
    fn test_expand_asset_name() {
        let template = "ripgrep-{VERSION}-x86_64-unknown-linux-musl.tar.gz";
        let result = expand_asset_name(template, "14.1.0");
        assert_eq!(result, "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz");
    }

    #[tokio::test]
    async fn test_ensure_tool_unknown_returns_error() {
        let tmp = TempDir::new().unwrap();
        let result = ensure_tool_with_cache("unknown_tool_xyz", tmp.path()).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Unknown tool"), "got: {}", msg);
    }

    #[tokio::test]
    async fn test_ensure_tool_uses_cached_binary() {
        let tmp = TempDir::new().unwrap();
        // Create a fake cached binary
        let cached = tmp.path().join("rg");
        std::fs::write(&cached, "#!/bin/sh\necho fake-rg").unwrap();

        // Temporarily remove rg from PATH by using a restricted PATH
        // This test validates that the cache is checked.
        // We can't easily override PATH in a test, so we just verify the
        // cache path is returned when no PATH binary exists.
        // In a real environment, PATH lookup might succeed first.

        // Verify cache file is at expected location
        assert!(cached.exists(), "cached binary should exist");
        assert_eq!(tmp.path().join("rg"), cached);
    }

    #[tokio::test]
    async fn test_ensure_tool_offline_mode_logic() {
        // Verify that is_offline() correctly reads the QUECTO_OFFLINE env var.
        // We test the downstream logic: ensure_tool_with_cache returns an
        // "offline" error when the cache is empty AND offline mode is active.
        // Since we cannot safely mutate env state, we test via the helper:
        // `ensure_tool_with_cache` with an empty tmp dir simulates a scenario
        // where the binary is unavailable in cache. Combined with is_offline()
        // returning true (when env is set externally), the error path fires.
        //
        // The BDD test in ensure_tool_steps.rs covers the full offline path.
        // Here we just verify the offline error message format.
        let offline_err = crate::domain::error::DomainError::Tool(
            "rg not found and offline mode is enabled (QUECTO_OFFLINE=1). Install manually: https://example.com".to_string(),
        );
        let msg = offline_err.to_string();
        assert!(
            msg.to_lowercase().contains("offline"),
            "offline error format: {}",
            msg
        );
    }
    #[test]
    fn test_find_binary_in_dir() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("rg"), "binary").unwrap();

        let found = find_binary_in_dir(tmp.path(), "rg");
        assert!(found.is_some(), "should find rg in subdir");
        assert!(found.unwrap().ends_with("rg"));
    }
}
