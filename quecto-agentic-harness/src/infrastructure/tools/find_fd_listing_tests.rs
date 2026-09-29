//! What find lists and how it shows it: VCS internals (#2199), the entry
//! kind (#2200) and workspace-relative paths (#2203).
use super::tests::{assert_reaped, at, fixture};
use super::*;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};

fn request(pattern: &str) -> FindPathsRequest {
    FindPathsRequest {
        pattern: pattern.into(),
        path: ".".into(),
        limit: 1000,
        kind: None,
    }
}

/// #2199: VCS internals are skipped below the root; a root inside one
/// still lists it, shown relative to the workspace (#2203).
#[tokio::test]
async fn vcs_internals_are_skipped_unless_searched_directly() {
    let dir = TempDir::new().unwrap();
    for directory in [
        ".git/objects",
        ".github",
        "src",
        "sub/.hg",
        "sub/.svn",
        "sub/.jj",
    ] {
        std::fs::create_dir_all(dir.path().join(directory)).unwrap();
    }
    for name in [
        ".git/HEAD",
        ".git/objects/x",
        "src/a.rs",
        ".env",
        ".github/ci.yml",
        ".gitignore",
        ".gitmodules",
        "sub/.hg/store",
        "sub/.svn/entries",
        "sub/.jj/repo",
    ] {
        std::fs::write(dir.path().join(name), "").unwrap();
    }
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    let everything = effect.find(request("*")).await.unwrap();
    // Only the VCS directories themselves go: names that merely start
    // with ".git" stay.
    assert_eq!(
        everything.entries,
        [
            ".env",
            ".github/",
            ".github/ci.yml",
            ".gitignore",
            ".gitmodules",
            "src/",
            "src/a.rs",
            "sub/"
        ]
    );
    assert_eq!(everything.skipped_vcs_dir.as_deref(), Some(".git"));
    let mut inside = request("*");
    inside.path = ".git".into();
    let inside = effect.find(inside).await.unwrap();
    assert_eq!(
        inside.entries,
        [".git/HEAD", ".git/objects/", ".git/objects/x"]
    );
}

/// #2200: the entry kind reaches fd, and '*' with it lists one kind.
#[tokio::test]
async fn the_entry_kind_filters_real_fd_output() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src/domain")).unwrap();
    std::fs::write(dir.path().join("src/domain/a.rs"), "").unwrap();
    std::fs::write(dir.path().join("top.rs"), "").unwrap();
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    for (kind, pattern, expected) in [
        (
            Some(FindEntryKind::Directory),
            "*",
            vec!["src/", "src/domain/"],
        ),
        (
            Some(FindEntryKind::File),
            "*",
            vec!["src/domain/a.rs", "top.rs"],
        ),
        (Some(FindEntryKind::Directory), "src/*", vec!["src/domain/"]),
        (
            None,
            "*",
            vec!["src/", "src/domain/", "src/domain/a.rs", "top.rs"],
        ),
    ] {
        let mut req = request(pattern);
        req.kind = kind;
        assert_eq!(
            effect.find(req).await.unwrap().entries,
            expected,
            "{kind:?} {pattern}"
        );
    }
}

#[test]
fn the_entry_kind_is_explicit_argv() {
    let effect = FdFindPaths::new(Arc::new(PathBuf::from("/ws")), Arc::new(Sandbox::new(None)));
    for (kind, expected) in [
        (None, vec![]),
        (Some(FindEntryKind::File), vec!["f", "l"]),
        (Some(FindEntryKind::Directory), vec!["d", "l"]),
    ] {
        let mut req = request("*");
        req.kind = kind;
        let command = effect.command(&req, Path::new("/ws"));
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        // Symlinks are asked for too, then kept by their target's kind.
        let types: Vec<_> = args
            .windows(2)
            .filter(|pair| pair[0] == "--type")
            .map(|pair| pair[1])
            .collect();
        assert_eq!(types, expected, "{kind:?}");
    }
}

/// #2203: entries are shown relative to the workspace, whatever path was
/// searched, so read and edit take them unchanged.
#[tokio::test]
async fn entries_are_relative_to_the_workspace() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("crate/src/domain")).unwrap();
    std::fs::write(dir.path().join("crate/src/domain/agent.rs"), "").unwrap();
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    let absolute = dir.path().join("crate/src").display().to_string();
    for path in [
        ".",
        "crate",
        "crate/src/domain",
        "./crate/src/domain/",
        "crate/../crate/src",
        absolute.as_str(),
    ] {
        let mut req = request("*.rs");
        req.path = path.into();
        assert_eq!(
            effect.find(req).await.unwrap().entries,
            ["crate/src/domain/agent.rs"],
            "{path}"
        );
    }
}

#[test]
fn a_search_root_is_shown_as_read_would_take_it() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir_all(workspace.path().join("real/dir")).unwrap();
    std::os::unix::fs::symlink(workspace.path().join("real"), workspace.path().join("link"))
        .unwrap();
    let ws = workspace.path();
    let outside_shown = format!(
        "{}/",
        std::fs::canonicalize(outside.path()).unwrap().display()
    );
    for (root, shown) in [
        (ws.join("."), String::new()),
        (ws.to_path_buf(), String::new()),
        (ws.join("real"), "real/".to_string()),
        (ws.join("real/./dir/"), "real/dir/".to_string()),
        // A symlink inside the workspace is kept as named.
        (ws.join("link/dir"), "link/dir/".to_string()),
        // '..' is resolved on disk, never by text.
        (ws.join("link/.."), String::new()),
        (ws.join("real/dir/../dir"), "real/dir/".to_string()),
        (outside.path().join("."), outside_shown.clone()),
        (PathBuf::from("/"), "/".to_string()),
    ] {
        assert_eq!(
            SearchRoot::new(root.clone(), ws, None).shown,
            shown,
            "{}",
            root.display()
        );
    }
}

#[test]
fn normalization_prefixes_the_shown_root() {
    let root = SearchRoot {
        searched: PathBuf::from("/ws/./src"),
        shown: "src/".into(),
        workspace: PathBuf::from("/ws"),
        kind: None,
        skipped_vcs_dir: None,
    };
    assert_eq!(
        normalize_output(b"/ws/./src/a.rs\0/ws/./src/d/\0/elsewhere\0", &root, false),
        ["src/a.rs", "src/d/", "/elsewhere"]
    );
}

/// #2200 review: a symlink counts as what it points at, so a linked
/// directory is a directory and a linked file a file; a dangling link is
/// neither. A symlink never ends in '/', as without a type.
#[tokio::test]
async fn symlinks_are_listed_by_their_targets_kind() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src/real")).unwrap();
    std::fs::write(dir.path().join("src/file.rs"), "").unwrap();
    for (target, link) in [
        ("real", "dlink"),
        ("file.rs", "flink"),
        ("nowhere", "broken"),
    ] {
        std::os::unix::fs::symlink(target, dir.path().join("src").join(link)).unwrap();
    }
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    for (kind, expected) in [
        (
            FindEntryKind::Directory,
            vec!["src/", "src/dlink", "src/real/"],
        ),
        (FindEntryKind::File, vec!["src/file.rs", "src/flink"]),
    ] {
        let mut req = request("*");
        req.kind = Some(kind);
        assert_eq!(
            effect.find(req).await.unwrap().entries,
            expected,
            "{kind:?}"
        );
    }
}

/// Review 2, demo A: a dangling link fd lists under type f is not a kept
/// entry, so it neither fills the limit nor claims more exist.
#[tokio::test]
async fn a_dropped_symlink_never_counts_toward_the_limit() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "").unwrap();
    std::os::unix::fs::symlink("nowhere", dir.path().join("broken")).unwrap();
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    let mut req = request("*");
    req.kind = Some(FindEntryKind::File);
    req.limit = 1;
    let result = effect.find(req).await.unwrap();
    assert_eq!(result.entries, ["a.txt"]);
    assert!(!result.result_limit_reached, "{result:?}");
}

/// Review 2, demo B: symlinks of the other kind cannot use up fd's budget
/// and hide the one file there is.
#[tokio::test]
async fn symlinks_of_the_other_kind_never_hide_an_entry() {
    let dir = TempDir::new().unwrap();
    for i in 0..30 {
        std::fs::create_dir(dir.path().join(format!("d{i:02}"))).unwrap();
        std::os::unix::fs::symlink(format!("d{i:02}"), dir.path().join(format!("l{i:02}")))
            .unwrap();
    }
    std::fs::write(dir.path().join("zz.txt"), "").unwrap();
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    let mut req = request("*");
    req.kind = Some(FindEntryKind::File);
    req.limit = 5;
    let result = effect.find(req).await.unwrap();
    assert_eq!(result.entries, ["zz.txt"]);
    assert!(!result.result_limit_reached, "{result:?}");
    // Directories under the same limit: 30 real + 30 linked, so more exist.
    let mut req = request("*");
    req.kind = Some(FindEntryKind::Directory);
    req.limit = 5;
    let result = effect.find(req).await.unwrap();
    assert_eq!(result.entries.len(), 5);
    assert!(result.result_limit_reached);
    assert!(!result.incomplete);
}

/// With a kind, fd has no result budget: the listing stops it once one past
/// the limit is kept, and the call ends with fd reaped.
#[tokio::test]
async fn enough_kept_entries_stop_fd() {
    let (dir, effect) = fixture(
        "printf %s $$ > pid\n: > kept\nkept=\"$(pwd -P)/kept\"\nwhile :; do printf '%s\\0' \"$kept\"; done",
    );
    let mut req = request("*");
    req.kind = Some(FindEntryKind::File);
    req.limit = 3;
    let result = timeout(Duration::from_secs(5), effect.find(req))
        .await
        .expect("enough entries end the call")
        .unwrap();
    assert_eq!(result.entries.len(), 3);
    assert!(result.result_limit_reached);
    assert!(!result.incomplete, "stopped for enough, not at the cap");
    assert_reaped(&dir).await;
}

#[test]
fn the_listing_stops_at_one_past_the_limit_and_at_the_cap() {
    let root = at("/ws");
    let mut listing = Listing::new(&root, 2, 64);
    assert_eq!(listing.feed(b"/ws/a\0/ws/b"), None);
    assert_eq!(listing.feed(b"\0/ws/c\0/ws/d\0"), Some(Stop::Enough));
    assert_eq!(listing.into_parts().0, ["a", "b", "c"]);
    // The cap is charged for kept entries only ("a" and its separator).
    let mut listing = Listing::new(&root, 100, 7);
    assert_eq!(listing.feed(b"/ws/a\0/ws/bcdef\0"), Some(Stop::Capped));
    assert_eq!(listing.seen(), 16);
    assert_eq!(
        listing.into_parts().0,
        ["a"],
        "an entry past the cap is dropped"
    );
    let mut listing = Listing::new(&root, 100, 64);
    assert_eq!(listing.feed(b"/ws/a\0/ws/last"), None);
    assert_eq!(listing.finish(), None);
    assert_eq!(listing.into_parts().0, ["a", "last"]);
}

#[tokio::test]
async fn a_doubled_slash_in_the_path_is_not_shown() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    for path in ["src//", "src///", "./src//"] {
        let mut req = request("*.rs");
        req.path = path.into();
        assert_eq!(
            effect.find(req).await.unwrap().entries,
            ["src/a.rs"],
            "{path}"
        );
    }
}

/// #2199 review: the skipped directory to suggest is the real one. In a
/// worktree `.git` is a file naming it; with none there is no suggestion.
#[test]
fn the_skipped_vcs_dir_is_the_real_one() {
    let workspace = TempDir::new().unwrap();
    let ws = workspace.path();
    for directory in [
        "repo/.git",
        "main/.git/worktrees/wt",
        "wt",
        "hg/.hg",
        "plain",
        "bad",
    ] {
        std::fs::create_dir_all(ws.join(directory)).unwrap();
    }
    std::fs::write(ws.join("wt/.git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
    std::fs::write(ws.join("bad/.git"), "gitdir: ../missing\n").unwrap();
    for (directory, named) in [("sub", "gitdir: inner\n"), ("etc", "gitdir: /etc\n")] {
        std::fs::create_dir_all(ws.join(directory).join("inner")).unwrap();
        std::fs::write(ws.join(directory).join(".git"), named).unwrap();
    }
    std::fs::create_dir_all(ws.join("sneaky/.git")).unwrap();
    std::fs::create_dir_all(ws.join("escape")).unwrap();
    std::fs::write(
        ws.join("escape/.git"),
        "gitdir: ../sneaky/.git/../../outside\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws.join("outside")).unwrap();
    std::fs::create_dir(ws.join("filed")).unwrap();
    std::fs::write(ws.join("notadir"), "").unwrap();
    std::fs::write(ws.join("filed/.git"), "gitdir: ../notadir\n").unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir(outside.path().join(".git")).unwrap();
    let absolute = format!("{}/.git", outside.path().display());
    for (root, expected) in [
        (ws.join("repo"), Some("repo/.git")),
        (ws.join("wt"), Some("main/.git/worktrees/wt")),
        (ws.join("hg"), Some("hg/.hg")),
        (ws.join("plain"), None),
        // A gitdir: that is missing, a file, or no VCS directory is not
        // believed: the .git file itself is named.
        (ws.join("bad"), Some("bad/.git")),
        (ws.join("filed"), Some("filed/.git")),
        (ws.join("sub"), Some("sub/.git")),
        (ws.join("etc"), Some("etc/.git")),
        (ws.join("escape"), Some("escape/.git")),
        (outside.path().to_path_buf(), Some(absolute.as_str())),
    ] {
        assert_eq!(
            skipped_vcs_dir(&root, ws).as_deref(),
            expected,
            "{}",
            root.display()
        );
    }
}

/// Review 3 (M1): records of the other kind never use up the byte cap, so
/// 4000 file symlinks (over 100 KB of fd output) cannot hide directories.
#[tokio::test]
async fn symlinks_of_the_other_kind_never_use_up_the_byte_cap() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("target.txt"), "").unwrap();
    for d in 0..200 {
        let sub = dir
            .path()
            .join(format!("directory-with-a-long-name-{d:03}"));
        std::fs::create_dir(&sub).unwrap();
        for l in 0..20 {
            std::os::unix::fs::symlink(
                "../target.txt",
                sub.join(format!("a-rather-long-symlink-name-{l:02}")),
            )
            .unwrap();
        }
    }
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    let mut req = request("*");
    req.kind = Some(FindEntryKind::Directory);
    req.limit = 10;
    let result = effect.find(req).await.unwrap();
    assert_eq!(result.entries.len(), 10, "{result:?}");
    assert!(result.entries.iter().all(|entry| entry.ends_with('/')));
    assert!(result.result_limit_reached);
    assert!(!result.incomplete, "{result:?}");
    // Every directory fits when the limit allows it.
    let mut req = request("*");
    req.kind = Some(FindEntryKind::Directory);
    req.limit = 1000;
    let result = effect.find(req).await.unwrap();
    assert_eq!(result.entries.len(), 200);
    assert!(!result.result_limit_reached && !result.incomplete);
}

/// Review 3 (M2): names are looked up by their bytes and each is one line:
/// a non-UTF-8 name is still a file, and a newline in a name is escaped,
/// never a phantom entry.
#[tokio::test]
async fn odd_names_are_looked_up_by_their_bytes_and_shown_on_one_line() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new().unwrap();
    let bad = std::ffi::OsStr::from_bytes(b"bad\xff.txt");
    std::fs::write(dir.path().join(bad), "").unwrap();
    std::fs::create_dir(dir.path().join("nl\nfake")).unwrap();
    std::fs::write(dir.path().join("x\ny"), "").unwrap();
    let effect = FdFindPaths::new(Arc::new(dir.path().into()), Arc::new(Sandbox::new(None)));
    for (kind, expected) in [
        (None, vec!["bad\u{fffd}.txt", "nl\\nfake/", "x\\ny"]),
        (Some(FindEntryKind::File), vec!["bad\u{fffd}.txt", "x\\ny"]),
        (Some(FindEntryKind::Directory), vec!["nl\\nfake/"]),
    ] {
        let mut req = request("*");
        req.kind = kind;
        assert_eq!(
            effect.find(req).await.unwrap().entries,
            expected,
            "{kind:?}"
        );
    }
}

#[test]
fn a_record_longer_than_any_path_stops_the_listing() {
    let root = at("/ws");
    let mut listing = Listing::new(&root, 100, 1 << 20);
    assert_eq!(listing.feed(&vec![b'x'; RECORD_MAX]), None);
    assert_eq!(listing.feed(b"x"), Some(Stop::Capped));
    let mut listing = Listing::new(&root, 100, 1 << 20);
    assert_eq!(listing.feed(&vec![b'x'; RECORD_MAX]), None);
    assert_eq!(listing.finish(), None);
    assert_eq!(
        listing.into_parts().0.len(),
        1,
        "a record of RECORD_MAX is taken"
    );
}

/// Kept entries that fill the cap exactly are all kept.
#[test]
fn entries_filling_the_cap_exactly_are_kept() {
    let root = at("/ws");
    let mut listing = Listing::new(&root, 100, 4);
    assert_eq!(listing.feed(b"/ws/a\0/ws/b\0"), None);
    assert_eq!(listing.feed(b"/ws/c\0"), Some(Stop::Capped));
    assert_eq!(listing.into_parts().0, ["a", "b"]);
}
