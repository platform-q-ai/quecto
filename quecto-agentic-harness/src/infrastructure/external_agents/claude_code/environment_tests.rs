use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::{
    CREDENTIAL_VARIABLES, INHERITED_VARIABLES, MEMBER_CLAUDE_CONFIG_DIR, MEMBER_DIR_OPEN_FLAGS,
    MEMBER_HOME_DIR, MemberEnvironment, MemberEnvironmentError, PRIVATE_DIR_MODE,
    PRIVATE_DIR_OPEN_FLAGS, effective_uid, member_environment, open_directory, open_flags,
    plain_leaf,
};
use crate::application::external_agent::dto::CredentialEnv;

fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

fn api_key(value: &str) -> CredentialEnv {
    CredentialEnv {
        name: "ANTHROPIC_API_KEY".into(),
        value: value.into(),
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Every name the parent session or the operator can leak, by name.
const DROPPED: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CONFIG_DIR",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "GH_TOKEN",
    "QUECTO_BASE_DIR",
    "QUECTO_SWARM_MEMBER",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "XDG_CONFIG_HOME",
    "SSH_AUTH_SOCK",
];

#[test]
fn only_allowlisted_variables_reach_claude() {
    let member = tempfile::tempdir().unwrap();
    let mut parent = vars(&[
        ("HOME", "/home/operator"),
        ("ANTHROPIC_API_KEY", "the-operators-key"),
    ]);
    for name in INHERITED_VARIABLES {
        parent.push((
            OsString::from(name),
            OsString::from(format!("parent-{name}")),
        ));
    }
    parent.push((OsString::from("PATH"), OsString::from("/usr/bin:/bin")));
    for name in DROPPED {
        parent.push((OsString::from(name), OsString::from(format!("leak-{name}"))));
    }
    let env = member_environment(&parent, member.path(), &api_key("member-key")).unwrap();

    let names: BTreeSet<&str> = env.names().into_iter().collect();
    let expected: BTreeSet<&str> = INHERITED_VARIABLES
        .iter()
        .copied()
        .chain(["HOME", "CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY"])
        .collect();
    assert_eq!(names, expected);
    assert_eq!(env.names().len(), expected.len(), "no name is given twice");
    for name in DROPPED {
        assert!(
            env.get(name).is_none() || *name == "CLAUDE_CONFIG_DIR",
            "{name} leaked"
        );
        assert!(
            env.variables
                .iter()
                .all(|(_, value)| value != &OsString::from(format!("leak-{name}"))),
            "the parent's {name} value leaked"
        );
    }
    assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "member-key");
    assert_eq!(env.get("PATH").unwrap(), "/usr/bin:/bin");
    assert_eq!(
        env.get("HOME").unwrap(),
        member.path().join(MEMBER_HOME_DIR).as_os_str()
    );
    assert_eq!(
        env.get("CLAUDE_CONFIG_DIR").unwrap(),
        member.path().join(MEMBER_CLAUDE_CONFIG_DIR).as_os_str()
    );
    assert!(!format!("{env:?}").contains("member-key"), "{env:?}");
}

#[test]
fn an_inherited_variable_the_parent_lacks_is_simply_absent() {
    let member = tempfile::tempdir().unwrap();
    let env = member_environment(&vars(&[("PATH", "/bin")]), member.path(), &api_key("k")).unwrap();
    assert_eq!(
        env.names(),
        vec!["PATH", "HOME", "CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY"]
    );
}

#[test]
fn home_and_config_dir_are_per_member_and_private() {
    let base = tempfile::tempdir().unwrap();
    let parent = vars(&[("PATH", "/bin")]);
    let one = member_environment(&parent, &base.path().join("members/one"), &api_key("k")).unwrap();
    let two = member_environment(&parent, &base.path().join("members/two"), &api_key("k")).unwrap();
    for env in [&one, &two] {
        for dir in [&env.home, &env.config_dir] {
            assert!(dir.is_dir(), "{} is made before spawn", dir.display());
            assert_eq!(mode(dir), PRIVATE_DIR_MODE, "{}", dir.display());
        }
    }
    assert_ne!(one.home, two.home);
    assert_ne!(one.config_dir, two.config_dir);
    assert!(one.home.starts_with(base.path().join("members/one")));
    assert!(one.config_dir.starts_with(base.path().join("members/one")));
}

#[test]
fn an_existing_private_dir_is_made_owner_only() {
    let member = tempfile::tempdir().unwrap();
    let home = member.path().join(MEMBER_HOME_DIR);
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = member_environment(&vars(&[]), member.path(), &api_key("k")).unwrap();
    assert_eq!(mode(&env.home), PRIVATE_DIR_MODE);
}

#[test]
fn a_symlinked_private_dir_is_refused() {
    let member = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(
        elsewhere.path(),
        member.path().join(MEMBER_CLAUDE_CONFIG_DIR),
    )
    .unwrap();
    let error = member_environment(&vars(&[]), member.path(), &api_key("k")).unwrap_err();
    assert!(
        matches!(error, MemberEnvironmentError::Directory { .. }),
        "{error:?}"
    );
}

#[test]
fn a_relative_member_dir_is_refused() {
    let error =
        member_environment(&vars(&[]), Path::new("members/one"), &api_key("k")).unwrap_err();
    assert!(
        matches!(error, MemberEnvironmentError::Directory { .. }),
        "{error:?}"
    );
}

#[test]
fn only_a_known_nonempty_credential_is_given() {
    let member = tempfile::tempdir().unwrap();
    for name in CREDENTIAL_VARIABLES {
        let credential = CredentialEnv {
            name: name.to_string(),
            value: "v".into(),
        };
        let env = member_environment(&vars(&[]), member.path(), &credential).unwrap();
        assert_eq!(env.get(name).unwrap(), "v");
    }
    for credential in [
        CredentialEnv {
            name: "GH_TOKEN".into(),
            value: "v".into(),
        },
        CredentialEnv {
            name: "PATH".into(),
            value: "/evil".into(),
        },
        api_key(""),
    ] {
        let error = member_environment(&vars(&[]), member.path(), &credential).unwrap_err();
        assert!(
            matches!(error, MemberEnvironmentError::Credential(_)),
            "{credential:?}: {error:?}"
        );
    }
}

#[test]
fn the_login_proxy_and_ca_variables_pass_through() {
    let member = tempfile::tempdir().unwrap();
    let passed = [
        "USER",
        "LOGNAME",
        "XDG_RUNTIME_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "ALL_PROXY",
        "all_proxy",
        "TZ",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
    ];
    let parent: Vec<(OsString, OsString)> = passed
        .iter()
        .map(|name| (OsString::from(name), OsString::from(format!("v-{name}"))))
        .collect();
    let env = member_environment(&parent, member.path(), &api_key("k")).unwrap();
    for name in passed {
        assert!(INHERITED_VARIABLES.contains(&name), "{name} is allowlisted");
        assert_eq!(
            env.get(name).unwrap(),
            OsString::from(format!("v-{name}")).as_os_str(),
            "{name}"
        );
    }
}

#[test]
fn a_symlinked_member_dir_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let member = base.path().join("member");
    std::os::unix::fs::symlink(elsewhere.path(), &member).unwrap();
    let error = member_environment(&vars(&[]), &member, &api_key("k")).unwrap_err();
    assert!(
        matches!(&error, MemberEnvironmentError::Directory { path, .. } if path == &member),
        "{error:?}"
    );
    assert!(
        std::fs::read_dir(elsewhere.path())
            .unwrap()
            .next()
            .is_none(),
        "nothing was made through the symlink"
    );
}

#[test]
fn a_member_dir_others_can_write_is_refused() {
    for loose in [0o770, 0o707, 0o777] {
        let base = tempfile::tempdir().unwrap();
        let member = base.path().join("member");
        std::fs::create_dir(&member).unwrap();
        std::fs::set_permissions(&member, std::fs::Permissions::from_mode(loose)).unwrap();
        let error = member_environment(&vars(&[]), &member, &api_key("k")).unwrap_err();
        assert!(
            matches!(&error, MemberEnvironmentError::Directory { path, .. } if path == &member),
            "{loose:o}: {error:?}"
        );
        assert!(
            !member.join(MEMBER_HOME_DIR).exists(),
            "{loose:o}: no private dir was made in it"
        );
    }
}

#[test]
fn a_member_dir_only_its_owner_writes_is_kept_as_it_is() {
    let base = tempfile::tempdir().unwrap();
    let member = base.path().join("member");
    std::fs::create_dir(&member).unwrap();
    std::fs::set_permissions(&member, std::fs::Permissions::from_mode(0o755)).unwrap();
    member_environment(&vars(&[]), &member, &api_key("k")).unwrap();
    assert_eq!(mode(&member), 0o755);
}

#[test]
fn debug_names_the_variables_and_directories_never_a_value() {
    let env = MemberEnvironment {
        variables: vec![
            ("PATH".into(), OsString::from("/secret-path")),
            (
                "ANTHROPIC_API_KEY".into(),
                OsString::from("sk-member-secret"),
            ),
        ],
        home: "/m/home".into(),
        config_dir: "/m/claude-config".into(),
    };
    assert_eq!(
        format!("{env:?}"),
        r#"MemberEnvironment { names: ["PATH", "ANTHROPIC_API_KEY"], home: "/m/home", config_dir: "/m/claude-config" }"#
    );
}

#[test]
fn a_private_dir_name_is_one_plain_component() {
    for plain in [MEMBER_HOME_DIR, MEMBER_CLAUDE_CONFIG_DIR, "a_b", "A9"] {
        assert!(plain_leaf(plain), "{plain:?}");
    }
    for refused in ["", ".", "..", "a/b", "/a", "a b", "a.b"] {
        assert!(!plain_leaf(refused), "{refused:?}");
    }
}

#[test]
fn the_open_flags_are_exactly_their_named_bits() {
    assert_eq!(
        open_flags(MEMBER_DIR_OPEN_FLAGS),
        libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC
    );
    assert_eq!(
        open_flags(PRIVATE_DIR_OPEN_FLAGS),
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC
    );
    for flags in [MEMBER_DIR_OPEN_FLAGS, PRIVATE_DIR_OPEN_FLAGS] {
        let nonzero: Vec<_> = flags.iter().copied().filter(|flag| *flag != 0).collect();
        for (at, flag) in nonzero.iter().enumerate() {
            assert_eq!(flag.count_ones(), 1, "{flag:#x} is one bit");
            assert!(
                nonzero[at + 1..].iter().all(|other| other & flag == 0),
                "{flag:#x} is its own bit"
            );
        }
    }
}

/// Whether `file`'s descriptor is closed on exec.
fn close_on_exec(file: &std::fs::File) -> bool {
    use std::os::fd::AsRawFd;
    // `file` owns an open descriptor for the whole call; `F_GETFD` only
    // reads its flags.
    // SAFETY: a valid, open descriptor; nothing is written through it.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0, "F_GETFD: {}", std::io::Error::last_os_error());
    flags & libc::FD_CLOEXEC != 0
}

#[test]
fn the_member_dir_opens_only_as_a_real_directory_closed_on_exec() {
    let base = tempfile::tempdir().unwrap();
    let directory = base.path().join("dir");
    std::fs::create_dir(&directory).unwrap();
    let opened = open_directory(&directory).unwrap();
    assert!(opened.metadata().unwrap().is_dir());
    assert!(close_on_exec(&opened), "no spawned child inherits it");

    let file = base.path().join("file");
    std::fs::write(&file, b"").unwrap();
    let link = base.path().join("link");
    std::os::unix::fs::symlink(&directory, &link).unwrap();
    for refused in [&file, &link] {
        let error = open_directory(refused).unwrap_err();
        assert!(
            matches!(&error, MemberEnvironmentError::Directory { path, .. } if path == refused),
            "{error:?}"
        );
    }
}

#[test]
fn a_private_dir_that_is_a_file_is_refused() {
    let member = tempfile::tempdir().unwrap();
    std::fs::write(member.path().join(MEMBER_HOME_DIR), b"").unwrap();
    let error = member_environment(&vars(&[]), member.path(), &api_key("k")).unwrap_err();
    assert!(
        matches!(
            &error,
            MemberEnvironmentError::Directory { path, detail }
                if path == &member.path().join(MEMBER_HOME_DIR)
                    && detail.starts_with("open as a directory")
        ),
        "{error:?}"
    );
}

#[test]
fn a_private_dir_that_cannot_be_made_is_refused_at_its_creation() {
    if effective_uid() == 0 {
        eprintln!("skipped: root may make a directory in a read-only one");
        return;
    }
    let base = tempfile::tempdir().unwrap();
    let member = base.path().join("member");
    std::fs::create_dir(&member).unwrap();
    std::fs::set_permissions(&member, std::fs::Permissions::from_mode(0o500)).unwrap();
    let outcome = member_environment(&vars(&[]), &member, &api_key("k"));
    std::fs::set_permissions(&member, std::fs::Permissions::from_mode(0o700)).unwrap();
    let error = outcome.unwrap_err();
    assert!(
        matches!(
            &error,
            MemberEnvironmentError::Directory { path, detail }
                if path == &member.join(MEMBER_HOME_DIR) && detail.starts_with("create: ")
        ),
        "only an existing directory is accepted, not any failed create: {error:?}"
    );
}
