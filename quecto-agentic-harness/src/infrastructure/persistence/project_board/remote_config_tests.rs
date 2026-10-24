use super::*;

/// A clone at `<dir>/work` whose `.git/config` is `config`.
fn clone_with(config: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join(".git")).unwrap();
    std::fs::write(work.join(".git/config"), config).unwrap();
    (dir, work)
}

const ORIGIN: &str = "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = /srv/r.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";

#[test]
fn a_remote_its_config_lists_with_a_url_is_listed() {
    let (_dir, work) = clone_with(ORIGIN);
    assert!(lists_remote(&work, "origin"));
}

#[test]
fn only_what_is_plainly_shown_lists_a_remote() {
    for (config, remote) in [
        (ORIGIN, "decoy"),
        (ORIGIN, "Origin"),
        ("[remote \"origin\"]\n\tfetch = x\n", "origin"),
        ("[remote \"origin\"]\n\t# url = /srv/r.git\n", "origin"),
        ("[remote \"origin\"]\n\turl =\n", "origin"),
        (
            "[remote \"other\"]\n\turl = /srv/r.git\n[remote \"origin\"]\n",
            "origin",
        ),
        (
            "[remote \"origin\"]\n[core]\n\turl = /srv/r.git\n",
            "origin",
        ),
        ("[include]\n\tpath = remotes.inc\n", "origin"),
        (ORIGIN, "../origin"),
        (ORIGIN, ""),
    ] {
        let (_dir, work) = clone_with(config);
        assert!(!lists_remote(&work, remote), "{remote:?} in {config:?}");
    }
}

#[test]
fn a_folder_that_is_no_clones_top_level_lists_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!lists_remote(dir.path(), "origin"), "no .git at all");
    let (_dir, work) = clone_with(ORIGIN);
    let sub = work.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    assert!(!lists_remote(&sub, "origin"), "a subdirectory is not shown");
    std::fs::remove_file(work.join(".git/config")).unwrap();
    std::fs::create_dir_all(work.join(".git/config")).unwrap();
    assert!(!lists_remote(&work, "origin"), "a config that is no file");
}

#[test]
fn a_worktree_reads_its_common_config() {
    let (dir, work) = clone_with(ORIGIN);
    let linked = dir.path().join("linked");
    let git_dir = work.join(".git/worktrees/linked");
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(git_dir.join("commondir"), "../..\n").unwrap();
    let pointer = format!("gitdir: {}\n", git_dir.display());
    std::fs::write(linked.join(".git"), pointer).unwrap();
    assert!(lists_remote(&linked, "origin"));
    assert!(!lists_remote(&linked, "decoy"));
}
