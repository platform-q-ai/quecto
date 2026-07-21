use crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

#[cfg(any(test, feature = "test-support"))]
static BUILT_PREVIEW_COUNTS_FOR_TESTS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn built_preview_count_for_tests(content: &str) -> usize {
    *BUILT_PREVIEW_COUNTS_FOR_TESTS
        .lock()
        .unwrap()
        .get(content)
        .unwrap_or(&0)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn reset_built_preview_count_for_tests(content: &str) {
    BUILT_PREVIEW_COUNTS_FOR_TESTS
        .lock()
        .unwrap()
        .insert(content.to_string(), 0);
}

pub(super) fn tool_result_preview(content: &str) -> String {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(count) = BUILT_PREVIEW_COUNTS_FOR_TESTS
        .lock()
        .unwrap()
        .get_mut(content)
    {
        *count += 1;
    }

    if content.len() > DEFAULT_OUTPUT_CAP_BYTES {
        let end = (0..=DEFAULT_OUTPUT_CAP_BYTES)
            .rev()
            .find(|&idx| content.is_char_boundary(idx))
            .unwrap_or(0);
        content[..end].to_string()
    } else {
        content.to_string()
    }
}

#[cfg(test)]
#[path = "agent_loop_preview_tests.rs"]
mod tests;
