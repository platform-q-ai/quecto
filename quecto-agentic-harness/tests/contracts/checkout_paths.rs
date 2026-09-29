//! `CheckoutPaths` on the filesystem adapter (#2275): a path is normalised
//! inside the checkout as Python's `resolve()` does it, and one resolving
//! outside is refused with the board's text.
use std::os::unix::fs::symlink;

use quecto::application::swarm::ports::CheckoutPaths;
use quecto::domain::swarm::BoardError;
use quecto::infrastructure::workspace::checkout_paths::ResolvedCheckout;

#[test]
fn paths_resolve_inside_the_checkout_or_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("checkout");
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::fs::create_dir_all(dir.path().join("outside")).unwrap();
    symlink(root.join("real"), root.join("alias")).unwrap();
    symlink(dir.path().join("outside"), root.join("out")).unwrap();
    let checkout = ResolvedCheckout::new(&root);
    assert_eq!(checkout.normalize("src/../a.rs"), Ok("a.rs".to_owned()));
    assert_eq!(checkout.normalize("alias/new"), Ok("real/new".to_owned()));
    assert_eq!(
        checkout.normalize("missing/x.rs"),
        Ok("missing/x.rs".to_owned())
    );
    for escape in ["../escape", "out/x", "/etc/passwd"] {
        assert_eq!(
            checkout.normalize(escape),
            Err(BoardError::new(
                "file must resolve inside the shared checkout"
            )),
            "{escape}"
        );
    }
}
