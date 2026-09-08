use super::*;

pub(super) async fn write_session_bytes_atomically(
    path: &Path,
    bytes: &[u8],
    rename_context: &str,
) -> Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;

    let tmp_path = path.with_extension("tmp");
    match tokio::fs::remove_file(&tmp_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(DomainError::Session(format!(
                "failed to write session: could not remove stale temp file: {error}"
            )));
        }
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp_path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to write session: {e}")))?;
    file.write_all(bytes)
        .await
        .map_err(|e| DomainError::Session(format!("failed to write session: {e}")))?;
    file.flush()
        .await
        .map_err(|e| DomainError::Session(format!("failed to flush session: {e}")))?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| DomainError::Session(format!("{rename_context}: {e}")))
}
