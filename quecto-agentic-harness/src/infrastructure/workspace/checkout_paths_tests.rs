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

/// `normalize(path)` for each `(path, expected)`: the relative path, or
/// the escape refusal for `None`.
fn assert_normalized(normalizer: &ResolvedCheckout, table: &[(&str, Option<&str>)]) {
    for &(path, expected) in table {
        let wanted = expected
            .map(str::to_owned)
            .ok_or_else(|| BoardError::new(RefusalKind::Invalid, ESCAPE));
        assert_eq!(normalizer.normalize(path), wanted, "{path}");
    }
}

/// A name is followed only when it is a symlink (#2321 mutation report):
/// the same name as a regular file is kept as written, so `..` after it
/// steps back to the checkout, while as a symlink to `src/inner` it steps
/// back to `src`. Pinned by running Python 3.14's `resolve()` over the
/// same tree.
#[test]
fn only_a_symlink_is_followed_not_a_regular_file_of_the_same_name() {
    let (_dir, _base, root) = checkout();
    std::fs::create_dir_all(root.join("src/inner")).unwrap();
    let normalizer = ResolvedCheckout::new(&root);
    std::fs::write(root.join("n"), "x").unwrap();
    assert_normalized(
        &normalizer,
        &[
            ("n/../x", Some("x")),
            ("n/x", Some("n/x")),
            ("n", Some("n")),
        ],
    );
    std::fs::remove_file(root.join("n")).unwrap();
    symlink("src/inner", root.join("n")).unwrap();
    assert_normalized(
        &normalizer,
        &[
            ("n/../x", Some("src/x")),
            ("n/x", Some("src/inner/x")),
            ("n", Some("src/inner")),
        ],
    );
}

/// Each `..` takes back one component of what is resolved so far: after
/// a symlink to `src/inner`, two steps reach the checkout and a third
/// leaves it; the same count on the written path agrees. Pinned by
/// Python 3.14's `resolve()`.
#[test]
fn each_parent_step_takes_back_one_resolved_component() {
    let (_dir, _base, root) = checkout();
    std::fs::create_dir_all(root.join("src/inner")).unwrap();
    symlink("src/inner", root.join("two")).unwrap();
    assert_normalized(
        &ResolvedCheckout::new(&root),
        &[
            ("two/../x", Some("src/x")),
            ("two/../../x", Some("x")),
            ("two/../../../x", None),
            ("src/inner/../../x", Some("x")),
            ("src/inner/../../../x", None),
        ],
    );
}

/// Non-strict `resolve()` follows a chain of any length (no hop limit) and
/// keeps a loop, reached through several links, as written. Pinned by
/// Python 3.14's `resolve()`.
#[test]
fn a_long_chain_resolves_and_a_multi_link_loop_is_kept() {
    let (_dir, _base, root) = checkout();
    for hop in 0..50 {
        let target = if hop < 49 {
            format!("c{}", hop + 1)
        } else {
            "real".to_owned()
        };
        symlink(target, root.join(format!("c{hop}"))).unwrap();
    }
    for (link, target) in [("h1", "h2"), ("h2", "h3"), ("h3", "h1")] {
        symlink(target, root.join(link)).unwrap();
    }
    assert_normalized(
        &ResolvedCheckout::new(&root),
        &[
            ("c0/new", Some("real/new")),
            ("c0", Some("real")),
            ("c25/../x", Some("x")),
            ("c49/new", Some("real/new")),
            ("h1/x", Some("h1/x")),
            ("h2", Some("h2")),
            ("h3/../x", Some("x")),
            ("h1/../x", Some("x")),
        ],
    );
}
