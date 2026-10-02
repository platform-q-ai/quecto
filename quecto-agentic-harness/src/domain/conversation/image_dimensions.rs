//! An image's pixel size from its header (#2420), read straight from the
//! base64 a conversation carries.

/// A pixel size; both sides are above 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// The pixel size of the `mime` image that `base64` encodes; `None` when
/// the header cannot be read.
pub fn image_dimensions(_mime: &str, _base64: &str) -> Option<Dimensions> {
    None
}

#[cfg(test)]
#[path = "image_dimensions_tests.rs"]
mod tests;
