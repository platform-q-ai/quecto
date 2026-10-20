use super::*;

/// The default store is `quecto/auth.json` under the user's data
/// directory, read from the environment as it is, never set by the test.
#[test]
fn the_default_location_is_quecto_auth_json_in_the_data_directory() {
    let path = AuthJsonFile::default_location().path;
    assert!(path.ends_with("quecto/auth.json"), "{}", path.display());
    let parent = path.parent().and_then(std::path::Path::parent);
    match dirs::data_dir() {
        Some(data_dir) => assert_eq!(parent, Some(data_dir.as_path())),
        None => assert_eq!(parent, Some(std::path::Path::new("~/.local/share"))),
    }
}

#[test]
fn a_store_at_a_path_reads_that_path() {
    let path = std::path::Path::new("/nonexistent/fake/auth.json");
    assert_eq!(AuthJsonFile::at(path).path, path);
}
