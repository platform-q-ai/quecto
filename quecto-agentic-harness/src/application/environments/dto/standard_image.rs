//! The default image tag of a project's standard container (#2073): named
//! after the project's directory, so two repositories on one machine build
//! two images instead of overwriting one shared tag.

use std::path::Path;

use super::STANDARD_CONTAINER_IMAGE;

/// Longest directory-derived part of the tag; an image name component may
/// be far longer, a readable one is not.
const MAX_NAME: usize = 100;

/// `quecto-<directory>:local`, with the directory name reduced to what an
/// image name admits: lowercase ASCII letters and digits, joined by a single
/// `.` or `_` where the name had exactly that, by `-` for anything else. A
/// name with nothing admissible falls back to the adapter's own default.
pub fn standard_image_for(project: &Path) -> String {
    let directory = project
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut name = String::new();
    let mut separator = String::new();
    for character in directory.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            if !name.is_empty() {
                name.push_str(match separator.as_str() {
                    "" => "",
                    "." => ".",
                    "_" => "_",
                    _ => "-",
                });
            }
            separator.clear();
            name.push(character);
        } else {
            separator.push(character);
        }
        if name.len() >= MAX_NAME {
            break;
        }
    }
    if name.is_empty() {
        return STANDARD_CONTAINER_IMAGE.to_string();
    }
    debug_assert!(name.is_ascii() && name.len() <= MAX_NAME);
    format!("quecto-{name}:local")
}
