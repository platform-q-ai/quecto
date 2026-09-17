use super::*;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

const PRETTY: &str = "{\n  \"unknown_key\": \"kept\",\n  \"agents\": {\n    \"defaults\": {\n      \"model\": \"old\"\n    }\n  }\n}\n";

fn patched() -> serde_json::Value {
    json!({"unknown_key":"kept","agents":{"defaults":{"model":"new"}}})
}

#[test]
fn a_file_in_the_writers_layout_changes_only_on_the_touched_line() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, PRETTY).unwrap();
    JsonDocumentWriter.write(&path, &patched()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        PRETTY.replace("\"old\"", "\"new\"")
    );
}

#[test]
fn the_existing_indentation_is_kept() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    for original in [PRETTY.replace("  ", "    "), PRETTY.replace("  ", "\t")] {
        std::fs::write(&path, &original).unwrap();
        write_document(&path, &patched()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("\"old\"", "\"new\"")
        );
    }
}

#[test]
fn a_compact_or_missing_file_is_written_with_two_space_indentation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("config.json");
    write_document(&path, &json!({"a":1})).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"a\": 1\n}\n"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600,
        "a new config file is private"
    );
    std::fs::write(&path, r#"{"a":1}"#).unwrap();
    write_document(&path, &json!({"a":1,"b":2})).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"a\": 1,\n  \"b\": 2\n}\n"
    );
    assert_eq!(detect_indent("{}"), None);
    assert_eq!(detect_indent("{\n\n  \"a\": 1\n}"), Some("  ".into()));
}

#[test]
fn an_existing_file_keeps_its_mode_and_no_temp_file_is_left_behind() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{}\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_document(&path, &json!({"a":1})).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn an_unreadable_existing_entry_or_an_unwritable_location_fails_with_the_reason() {
    let dir = TempDir::new().unwrap();
    let directory = dir.path().join("config.json");
    std::fs::create_dir(&directory).unwrap();
    assert!(write_document(&directory, &json!({})).is_err());
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    assert!(write_document(&blocker.join("config.json"), &json!({})).is_err());
}

#[test]
fn a_symlinked_config_is_written_through_the_link() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("dotfiles").join("quecto.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "{}\n").unwrap();
    let link = dir.path().join("config.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    write_document(&link, &json!({"a": 1})).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\n  \"a\": 1\n}\n"
    );

    let dangling = dir.path().join("dangling.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &dangling).unwrap();
    let error = write_document(&dangling, &json!({})).unwrap_err();
    assert!(error.contains("symlink"), "{error}");
}
