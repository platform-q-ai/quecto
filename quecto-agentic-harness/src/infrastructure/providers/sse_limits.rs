use crate::domain::error::DomainError;

/// Shared bounded append for provider SSE accumulators.
pub(crate) fn append_with_limit(
    target: &mut String,
    fragment: &str,
    limit: usize,
    label: &str,
) -> Result<(), DomainError> {
    let new_len = target
        .len()
        .checked_add(fragment.len())
        .ok_or_else(|| DomainError::Provider(format!("{label} exceeds {limit} byte limit")))?;
    if new_len > limit {
        return Err(DomainError::Provider(format!(
            "{label} exceeds {limit} byte limit"
        )));
    }
    target.push_str(fragment);
    Ok(())
}
