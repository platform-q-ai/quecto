//! The default image tag of a project's standard container (#2073): named
//! after the project's folder, so repositories in differently named folders
//! build their own images instead of overwriting one shared tag. Folders
//! that share a name (or reduce to the same name) share a tag: `--image`
//! tells them apart.

use std::path::Path;

use super::STANDARD_CONTAINER_IMAGE;

/// Longest folder-derived part of the tag; an image name component may be
/// far longer, a readable one is not.
const MAX_NAME: usize = 100;

/// `quecto-<folder>:local`, with the folder name reduced to what an image
/// name admits: lowercase ASCII letters and digits, joined by a single `.`
/// or `_` where the name had exactly that between them and by `-` for any
/// other run; leading and trailing runs are dropped; at most [`MAX_NAME`]
/// characters. A name with nothing admissible falls back to the adapter's
/// own default.
pub fn standard_image_for(project: &Path) -> String {
    let folder = project
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut name = String::new();
    let mut run = String::new();
    for character in folder.chars() {
        let admitted = character.is_ascii_lowercase() || character.is_ascii_digit();
        if admitted {
            let joiner = match (name.is_empty(), run.as_str()) {
                (true, _) | (false, "") => "",
                (false, ".") => ".",
                (false, "_") => "_",
                (false, _) => "-",
            };
            // The joiner and the character land together or not at all, so
            // the name never ends in a separator and never passes the cap.
            if name.len() + joiner.len() < MAX_NAME {
                name.push_str(joiner);
                name.push(character);
                run.clear();
            } else {
                break;
            }
        } else {
            run.push(character);
        }
    }
    if name.is_empty() {
        return STANDARD_CONTAINER_IMAGE.to_string();
    }
    assert!(
        name.len() <= MAX_NAME
            && name.starts_with(|c: char| c.is_ascii_alphanumeric())
            && name.ends_with(|c: char| c.is_ascii_alphanumeric()),
        "not an image name component: {name}"
    );
    format!("quecto-{name}:local")
}
