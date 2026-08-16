use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::AsyncReadExt;

use crate::domain::error::DomainError;

pub(crate) static EXEC_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
/// Deepest directory nesting the workspace walk will descend into. A program is
/// free to create a deeper tree; the walk stops rather than following it.
pub(crate) const SNAPSHOT_MAX_DEPTH: usize = 64;
/// Upper bound on entries recorded per snapshot, so a workspace with a very
/// large file count cannot make the walk unbounded in memory or time.
pub(crate) const SNAPSHOT_MAX_ENTRIES: usize = 20_000;

/// Walks the workspace iteratively rather than recursively: a program can nest
/// directories arbitrarily deep, and recursion here would overflow the worker
/// thread's stack, which aborts the whole process rather than unwinding.
pub(crate) fn snapshot_files(root: &Path) -> BTreeMap<String, SystemTime> {
    let mut m = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth >= SNAPSHOT_MAX_DEPTH || m.len() >= SNAPSHOT_MAX_ENTRIES {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            if m.len() >= SNAPSHOT_MAX_ENTRIES {
                return m;
            }
            let p = e.path();
            // The tool's own stdout/stderr artifacts live here; reporting them
            // as files the program wrote would be misleading.
            if p.strip_prefix(root).is_ok_and(is_reserved_artifact_rel) {
                continue;
            }
            match e.file_type() {
                // Symlinks are recorded but never descended into, so a link
                // back to an ancestor cannot make the walk cycle.
                Ok(ft) if ft.is_dir() => stack.push((p, depth + 1)),
                Ok(_) => {
                    if let (Ok(rel), Ok(md)) = (p.strip_prefix(root), e.metadata()) {
                        m.insert(
                            rel.to_string_lossy().to_string(),
                            md.modified().unwrap_or(UNIX_EPOCH),
                        );
                    }
                }
                Err(_) => continue,
            }
        }
    }
    m
}

/// Workspace-relative paths under the tool's own artifact directory.
pub(crate) fn is_reserved_artifact_rel(rel: &Path) -> bool {
    let mut parts = rel.components().map(|c| c.as_os_str());
    parts.next().is_some_and(|c| c == ".quecto") && parts.next().is_some_and(|c| c == "python_lab")
}

pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(component.as_os_str()),
        }
    }
    out
}

pub(crate) fn is_reserved_artifact_path(workspace: &Path, path: &Path) -> bool {
    let effective = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    let effective = lexical_normalize(&effective);
    let reserved = lexical_normalize(&workspace.join(".quecto/python_lab"));
    effective.starts_with(reserved)
}
pub(crate) fn changed_files(root: &Path, before: BTreeMap<String, SystemTime>) -> Vec<String> {
    snapshot_files(root)
        .into_iter()
        .filter(|(p, t)| before.get(p).map(|b| b < t).unwrap_or(true))
        .map(|(p, _)| p)
        .collect()
}
pub(crate) async fn truncation_marker_exists(path: &Path) -> Result<bool, DomainError> {
    let mut marker = path.to_path_buf();
    marker.set_extension("truncated");
    match tokio::fs::metadata(marker).await {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ioerr(e)),
    }
}

pub(crate) async fn read_preview(path: &Path, max: usize) -> Result<(String, bool), DomainError> {
    // A missing artifact reads as empty rather than failing the call: the
    // directory can legitimately be gone if another instance sharing this
    // workspace pruned it.
    let Ok(md) = tokio::fs::metadata(path).await else {
        return Ok((String::new(), false));
    };
    let trunc = md.len() as usize > max;
    let f = tokio::fs::File::open(path).await.map_err(ioerr)?;
    // A single read() is capped by tokio's internal buffer (2 MiB), which would
    // silently return a short preview while reporting it as complete.
    let mut buf = Vec::new();
    f.take(max as u64)
        .read_to_end(&mut buf)
        .await
        .map_err(ioerr)?;
    Ok((String::from_utf8_lossy(&buf).to_string(), trunc))
}
pub(crate) async fn read_slice(
    path: &Path,
    offset: usize,
    limit: usize,
) -> Result<(String, bool), DomainError> {
    use tokio::io::AsyncSeekExt;
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((String::new(), false)),
        Err(e) => return Err(ioerr(e)),
    };
    let len = metadata.len();
    if offset as u128 >= len as u128 {
        return Ok((String::new(), false));
    }
    let mut file = tokio::fs::File::open(path).await.map_err(ioerr)?;
    file.seek(std::io::SeekFrom::Start(offset as u64))
        .await
        .map_err(ioerr)?;
    let to_read = limit.min((len - offset as u64) as usize);
    let mut buf = Vec::new();
    file.take(to_read as u64)
        .read_to_end(&mut buf)
        .await
        .map_err(ioerr)?;
    let n = buf.len();
    Ok((
        String::from_utf8_lossy(&buf).to_string(),
        (offset as u64).saturating_add(n as u64) < len,
    ))
}
pub(crate) fn rel(workspace: &Path, p: &Path) -> String {
    p.strip_prefix(workspace)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}
pub(crate) fn artifact_rel(p: &Path) -> String {
    let parts: Vec<_> = p
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(i) = parts.iter().position(|x| x == ".quecto") {
        parts[i..].join("/")
    } else {
        p.to_string_lossy().to_string()
    }
}
pub(crate) fn bounded_u64(
    value: &serde_json::Value,
    key: &str,
    default: u64,
    maximum: u64,
) -> Result<u64, String> {
    match value.get(key) {
        None => Ok(default.min(maximum)),
        Some(v) => v
            .as_u64()
            .map(|n| n.min(maximum))
            .ok_or_else(|| format!("{key} must be a non-negative integer")),
    }
}
pub(crate) fn ioerr(e: std::io::Error) -> DomainError {
    DomainError::Other(e.to_string())
}
