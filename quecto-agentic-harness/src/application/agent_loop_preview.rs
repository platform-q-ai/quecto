use crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

pub(super) fn tool_result_preview(content: &str) -> String {
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
