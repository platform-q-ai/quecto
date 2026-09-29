use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::ResolvedCheckout;
use crate::application::swarm::ports::CheckoutPaths;
use crate::domain::swarm::{BoardError, RefusalKind};

const ESCAPE: &str = "file must resolve inside the shared checkout";

/// `<base>/checkout` holding `src/`, `real/`, a regular `file.txt`, and
/// the links `alias -> <root>/real`, `out -> <base>/outside`, `loop ->
/// loop2 -> loop`, `up -> ..` and `rel -> src/../real`; `<base>/link-root`
/// links to the checkout.
fn checkout() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let root = base.join("checkout");
    for folder in [root.join("src"), root.join("real"), base.join("outside")] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(root.join("file.txt"), "x").unwrap();
    for (target, link) in [
        (root.join("real"), root.join("alias")),
        (base.join("outside"), root.join("out")),
        (PathBuf::from("loop2"), root.join("loop")),
        (PathBuf::from("loop"), root.join("loop2")),
        (PathBuf::from(".."), root.join("up")),
        (PathBuf::from("src/../real"), root.join("rel")),
        (root.clone(), base.join("link-root")),
    ] {
        symlink(target, link).unwrap();
    }
    (dir, base, root)
}

fn inside(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

/// Python's `str((root / path).resolve().relative_to(root))`, pinned by
/// running Python 3.14 over the same tree: `resolve()` is non-strict, so a
/// missing remainder is kept as written after the existing prefix is
/// resolved (`..` taken back on the resolved path), a symlink loop or a
/// component under a regular file stays as written, and a path leaving
/// the root (by `..`, an absolute path, or a link) is refused.
#[test]
fn normalize_matches_python_resolve_for_missing_files() {
    let (_dir, base, root) = checkout();
    let normalizer = ResolvedCheckout::new(&root);
    let table: Vec<(String, Result<&str, &str>)> = vec![
        ("a/../b.rs".to_owned(), Ok("b.rs")),
        ("./x".to_owned(), Ok("x")),
        ("alias/new".to_owned(), Ok("real/new")),
        (
            "missing/deep/file.rs".to_owned(),
            Ok("missing/deep/file.rs"),
        ),
        ("../escape".to_owned(), Err(ESCAPE)),
        (inside(&root.join("src/a.rs")), Ok("src/a.rs")),
        (".".to_owned(), Ok(".")),
        ("src/".to_owned(), Ok("src")),
        ("out/x".to_owned(), Err(ESCAPE)),
        ("loop/x".to_owned(), Ok("loop/x")),
        ("up/checkout/a".to_owned(), Ok("a")),
        ("file.txt/x".to_owned(), Ok("file.txt/x")),
        ("a\0b".to_owned(), Err(ESCAPE)),
        ("rel/n".to_owned(), Ok("real/n")),
        ("missing/../a.rs".to_owned(), Ok("a.rs")),
        ("alias/../src/a.rs".to_owned(), Ok("src/a.rs")),
        (inside(&base.join("link-root/src/a.rs")), Ok("src/a.rs")),
        ("//x".to_owned(), Err(ESCAPE)),
        ("src//a.rs".to_owned(), Ok("src/a.rs")),
    ];
    for (path, expected) in table {
        let normalized = normalizer.normalize(&path);
        match expected {
            Ok(relative) => assert_eq!(normalized, Ok(relative.to_owned()), "{path:?}"),
            Err(message) => assert_eq!(
                normalized,
                Err(BoardError::new(RefusalKind::Invalid, message)),
                "{path:?}"
            ),
        }
    }
}

/// A checkout named through a symlink is resolved first, as Python
/// resolves `self.checkout`.
#[test]
fn a_linked_checkout_is_resolved_first() {
    let (_dir, base, _root) = checkout();
    let normalizer = ResolvedCheckout::new(base.join("link-root"));
    assert_eq!(normalizer.normalize("alias/new"), Ok("real/new".to_owned()));
    assert_eq!(
        normalizer.normalize(&inside(&base.join("link-root/src/a.rs"))),
        Ok("src/a.rs".to_owned())
    );
}

/// `non_utf8_resolved_path_is_refused` (#2275, PR #2321 review): a path
/// resolving, through a symlink a worker made, to a name that is not
/// UTF-8 is refused as an escape, where Python's `resolve()` answers a
/// surrogate escape its `sqlite3` then cannot encode.
#[test]
fn a_path_resolving_to_a_name_that_is_not_utf8_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let (_dir, _base, root) = checkout();
    symlink(OsStr::from_bytes(b"\xff"), root.join("l")).unwrap();
    let normalizer = ResolvedCheckout::new(&root);
    for path in ["l/x", "l", "src/../l/x"] {
        assert_eq!(
            normalizer.normalize(path),
            Err(BoardError::new(RefusalKind::Invalid, ESCAPE)),
            "{path}"
        );
    }
    assert_eq!(normalizer.normalize("src/x"), Ok("src/x".to_owned()));
}
