//! The fields a refusal over a file set aside names (#2659): the file by
//! its raw path and object, as `clear_set_aside` takes them, never
//! redacted (a name that merely looks like a secret is cleared by it).
use serde_json::{Map, Value, json};

use super::artifacts::artifact_folder;
use super::{ErrorCode, key_fields, slug_json};
use crate::application::project_board::ports::ArtifactKey;
use crate::domain::project_board::value_objects::shown_text::{
    shown_artifact_path, shown_board_path,
};

/// A file set aside, raw: its `path` (one a store shows sanitised) and
/// its `object` (git's id).
pub(super) fn file_fields(path: &str, object: &str) -> Map<String, Value> {
    assert!(
        shown_board_path(path),
        "a store shows every path it sets aside sanitised: {path:?}"
    );
    assert!(
        object.bytes().all(|b| b.is_ascii_hexdigit()),
        "a set-aside file's object is git's id"
    );
    let mut named = Map::new();
    named.insert("path".into(), json!(path));
    named.insert("object".into(), json!(object));
    named
}

/// The artifact a refusal names: a task as `set_aside` by its id, as
/// ever; any other kind as `artifact_set_aside` by [`key_fields`] and its
/// file's raw path, so a clear can name it exactly.
pub(super) fn fields_of(
    key: &ArtifactKey,
    path: &str,
    object: &str,
) -> (ErrorCode, Map<String, Value>) {
    let folder = match key {
        ArtifactKey::Task(id) => {
            let mut named = file_fields(path, object);
            named.insert("task".into(), slug_json(id));
            return (ErrorCode::SetAside, named);
        }
        ArtifactKey::Plan(..)
        | ArtifactKey::Run(..)
        | ArtifactKey::Change(..)
        | ArtifactKey::Review(..)
        | ArtifactKey::Discussion(_) => {
            artifact_folder(key).expect("every kind but a task has a folder")
        }
    };
    // Its own kind's folder, one name in it.
    assert!(
        shown_artifact_path(folder, path),
        "a store shows every path it sets aside sanitised: {path:?}"
    );
    let mut named = key_fields(key);
    named.extend(file_fields(path, object));
    (ErrorCode::ArtifactSetAside, named)
}
