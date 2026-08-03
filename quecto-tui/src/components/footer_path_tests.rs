use std::path::{Path, PathBuf};

fn abbreviate_for_home(path: &Path, home: &Path) -> String {
    if path == home {
        return "~".to_string();
    }
    if let Ok(rest) = path.strip_prefix(home) {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

#[test]
fn pwd_abbreviation_uses_path_components() {
    let home = PathBuf::from("/tmp/alice");

    assert_eq!(
        abbreviate_for_home(Path::new("/tmp/alice2/project"), &home),
        "/tmp/alice2/project"
    );
    assert_eq!(
        abbreviate_for_home(Path::new("/tmp/alice/project"), &home),
        "~/project"
    );
}
