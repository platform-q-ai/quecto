use crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

#[cfg(any(test, feature = "test-support"))]
static BUILT_PREVIEW_COUNT_FOR_TESTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn built_preview_count_for_tests() -> usize {
    BUILT_PREVIEW_COUNT_FOR_TESTS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn reset_built_preview_count_for_tests() {
    BUILT_PREVIEW_COUNT_FOR_TESTS.store(0, std::sync::atomic::Ordering::Relaxed);
}

pub(super) fn tool_result_preview(content: &str) -> String {
    #[cfg(any(test, feature = "test-support"))]
    BUILT_PREVIEW_COUNT_FOR_TESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
