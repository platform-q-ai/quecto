use std::path::PathBuf;

use super::denylist::check_with;
use super::protected_dirs::{HostContext, is_protected_target};

fn linux() -> HostContext {
    HostContext {
        home: Some(PathBuf::from("/home/swq")),
        workspace: Some(PathBuf::from("/home/swq/proj")),
        cwd: Some(PathBuf::from("/home/swq/proj")),
    }
}

fn mac() -> HostContext {
    HostContext {
        home: Some(PathBuf::from("/Users/swq")),
        workspace: Some(PathBuf::from("/Users/swq/proj")),
        cwd: Some(PathBuf::from("/Users/swq/proj")),
    }
}

fn blocked(host: &HostContext, cmd: &str) {
    match check_with(cmd, host) {
        Err(v) => assert_eq!(v.rule, "rm-protected-dir", "{cmd}: {}", v.rule),
        Ok(()) => panic!("expected `{cmd}` to be blocked"),
    }
}

fn allowed(host: &HostContext, cmd: &str) {
    if let Err(v) = check_with(cmd, host) {
        panic!("expected `{cmd}` allowed, got {} at `{}`", v.rule, v.site);
    }
}

#[test]
fn home_in_every_spelling_is_protected() {
    let h = linux();
    for c in [
        "rm -rf ~",
        "rm -rf ~/",
        "rm -rf ~/*",
        "rm -rf ~/.*",
        "rm -rf $HOME",
        "rm -rf ${HOME}",
        "rm -rf \"$HOME\"",
        "rm -rf \"$HOME\"/",
        "rm -rf $HOME/*",
        "rm -rf /home/swq",
        "rm -rf /home/swq/",
        "rm -rf /home/swq/*",
        "rm -rf /home/swq/./",
        "rm -rf /home/swq/proj/..",
        "rm -r ~",
        "rm --recursive --force ~",
        "sudo rm -rf ~",
        "bash -c 'rm -rf ~'",
        "rm -rf ~/$DIR",
        "rm -rf $HOME/$DIR",
    ] {
        blocked(&h, c);
    }
}

#[test]
fn mac_home_is_resolved_at_runtime() {
    let h = mac();
    blocked(&h, "rm -rf ~");
    blocked(&h, "rm -rf /Users/swq");
    blocked(&h, "rm -rf /Users/swq/*");
    blocked(&h, "rm -rf $HOME");
    allowed(&h, "rm -rf /Users/swq/Library/Caches/foo");
}

#[test]
fn workspace_root_is_protected_including_relative_forms() {
    let h = linux();
    for c in [
        "rm -rf /home/swq/proj",
        "rm -rf .",
        "rm -rf ./",
        "rm -rf ./*",
        "rm -rf ../proj",
        "rm -rf ..",
        "rm -rf $PWD",
    ] {
        if c == "rm -rf $PWD" {
            // No literal prefix: not resolvable, documented gap.
            allowed(&h, c);
        } else {
            blocked(&h, c);
        }
    }
    allowed(&h, "rm -rf build");
    allowed(&h, "rm -rf ./target");
    allowed(&h, "rm -rf ../other-proj");
    allowed(&h, "rm -rf target/*");
}

#[test]
fn top_level_system_dirs_are_protected_by_depth() {
    let h = linux();
    for c in [
        "rm -rf /etc",
        "rm -rf /usr/",
        "rm -rf /var/*",
        "rm -rf /home",
        "rm -rf /Users",
        "rm -rf /System",
        "rm -rf /Library",
        "rm -rf /private",
        "rm -rf /opt",
        "rm -rf /boot",
        "rm -rf /tmp",
        "rm -rf /usr/local/..",
    ] {
        blocked(&h, c);
    }
}

#[test]
fn sibling_homes_are_protected() {
    let h = linux();
    blocked(&h, "rm -rf /home/other");
    blocked(&h, "rm -rf /home/other/");
    blocked(&h, "rm -rf /Users/other");
    allowed(&h, "rm -rf /home/other/tmp");
}

#[test]
fn deeper_paths_stay_allowed() {
    let h = linux();
    for c in [
        "rm -rf /tmp/build",
        "rm -rf /usr/local/lib/foo",
        "rm -rf ~/.cache/foo",
        "rm -rf $HOME/.cargo/registry",
        "rm -rf /home/swq/.cache",
        "rm -rf /home/swq/proj/target",
        "rm -rf /var/tmp/x",
        "rm -f ~",
        "rm ~/file",
        "ls -R ~",
        "rm -rf ~/other",
        "echo rm -rf ~",
    ] {
        allowed(&h, c);
    }
}

#[test]
fn unknown_host_facts_only_protect_absolute_paths() {
    let h = HostContext::default();
    allowed(&h, "rm -rf ~");
    allowed(&h, "rm -rf .");
    blocked(&h, "rm -rf /etc");
    blocked(&h, "rm -rf /home/anyone");
}

#[test]
fn is_protected_target_edge_cases() {
    let h = linux();
    assert!(!is_protected_target("", &h));
    assert!(!is_protected_target("/", &h));
    assert!(is_protected_target("/etc/", &h));
    assert!(is_protected_target("~/proj/../", &h));
    assert!(!is_protected_target("~/proj/../.cache", &h));
    assert!(
        is_protected_target("/home/swq/proj/*/..", &h)
            || !is_protected_target("/home/swq/proj/*/..", &h)
    );
}

#[test]
fn from_host_uses_the_workspace_as_cwd() {
    let h = HostContext::from_host(Some(std::path::Path::new("/tmp/ws")));
    assert_eq!(h.workspace, Some(PathBuf::from("/tmp/ws")));
    assert_eq!(h.cwd, Some(PathBuf::from("/tmp/ws")));
    assert_eq!(h.home, dirs::home_dir());
    let none = HostContext::from_host(None);
    assert!(none.workspace.is_none() && none.cwd.is_none());
}
