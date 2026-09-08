use super::agents_instructions::load_agents_instructions;

#[test]
fn loads_only_agents_md_in_the_initialization_directory() {
    let parent = tempfile::tempdir().unwrap();
    let initialization_dir = parent.path().join("project");
    std::fs::create_dir(&initialization_dir).unwrap();
    std::fs::write(parent.path().join("AGENTS.md"), "parent instructions").unwrap();
    std::fs::write(
        initialization_dir.join("AGENTS.md"),
        "initialization instructions 🦀",
    )
    .unwrap();

    let loaded = load_agents_instructions(&initialization_dir).unwrap();

    assert_eq!(loaded.as_deref(), Some("initialization instructions 🦀"));
}

#[test]
fn missing_agents_md_is_a_no_op_even_when_an_ancestor_has_one() {
    let parent = tempfile::tempdir().unwrap();
    let initialization_dir = parent.path().join("project");
    std::fs::create_dir(&initialization_dir).unwrap();
    std::fs::write(parent.path().join("AGENTS.md"), "must not be inherited").unwrap();

    assert_eq!(load_agents_instructions(&initialization_dir).unwrap(), None);
}

#[test]
fn missing_initialization_directory_is_a_contextual_read_error() {
    let parent = tempfile::tempdir().unwrap();
    let initialization_dir = parent.path().join("missing-project");
    let path = initialization_dir.join("AGENTS.md");

    let error = load_agents_instructions(&initialization_dir).unwrap_err();

    assert!(error.contains("failed to read AGENTS.md"), "{error}");
    assert!(error.contains(&path.display().to_string()), "{error}");
}

#[cfg(unix)]
#[test]
fn dangling_agents_md_symlink_is_a_contextual_read_error() {
    use std::os::unix::fs::symlink;

    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    symlink(initialization_dir.path().join("missing-target"), &path).unwrap();

    let error = load_agents_instructions(initialization_dir.path()).unwrap_err();

    assert!(error.contains("failed to read AGENTS.md"), "{error}");
    assert!(error.contains(&path.display().to_string()), "{error}");
}

#[test]
fn read_error_names_the_exact_agents_md_path() {
    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    std::fs::create_dir(&path).unwrap();

    let error = load_agents_instructions(initialization_dir.path()).unwrap_err();

    assert!(error.contains("failed to read AGENTS.md"), "{error}");
    assert!(error.contains(&path.display().to_string()), "{error}");
}

#[test]
fn utf8_error_names_the_exact_agents_md_path() {
    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    std::fs::write(&path, [b'v', b'a', b'l', b'i', b'd', 0xff]).unwrap();

    let error = load_agents_instructions(initialization_dir.path()).unwrap_err();

    assert!(error.contains("AGENTS.md is not valid UTF-8"), "{error}");
    assert!(error.contains(&path.display().to_string()), "{error}");
}
