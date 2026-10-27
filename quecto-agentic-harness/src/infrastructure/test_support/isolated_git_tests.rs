use super::{checkout_with_origin, isolated_git};

/// The checkout's origin is written to its own config, and the global
/// configuration git reads is the temporary home's, which is empty.
#[test]
fn a_checkout_is_made_and_configured_inside_its_temporary_home() {
    let root = tempfile::tempdir().unwrap();
    let project = checkout_with_origin(root.path(), "https://git.example.test/o/r");
    let origin = isolated_git(
        root.path(),
        &project,
        &["config", "--show-origin", "--get", "remote.origin.url"],
    );
    let shown = String::from_utf8_lossy(&origin.stdout);
    assert!(
        shown.trim_end() == "file:.git/config\thttps://git.example.test/o/r",
        "{shown}"
    );
    let global = isolated_git(root.path(), &project, &["config", "--global", "--list"]);
    assert!(global.stdout.is_empty(), "no global configuration is read");
}
