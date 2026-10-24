//! #2659: a refusal over a file set aside names it raw.
use super::*;

/// #2659 review round 2: a set-aside file whose name looks like a secret
/// is named raw, path and object, so `clear_set_aside` can clear it; a
/// task set aside names its path too.
#[test]
fn a_set_aside_name_that_looks_like_a_secret_is_named_raw() {
    let path = "plans/ghp_abcdefghijklmnopqrstuvwxyz0123.1.md";
    assert_ne!(
        shown_json(path),
        json!(path),
        "the name looks like a secret"
    );
    let folder = error_json(&ProjectBoardError::Store(BoardStoreError::FolderSetAside {
        why: format!("{path} is set aside"),
        path: path.into(),
        object: "c".repeat(40),
    }));
    assert_eq!(
        (&folder["path"], &folder["object"]),
        (&json!(path), &json!("c".repeat(40)))
    );
    let task = error_json(&ProjectBoardError::Quarantined {
        key: ArtifactKey::Task(slug("a")),
        path: "tasks/a.md".into(),
        object: "c".repeat(40),
    });
    assert_eq!(task["path"], json!("tasks/a.md"));
}
