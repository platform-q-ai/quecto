//! git's command-scope configuration in the runner's environment: its
//! identity settings pass, renumbered, and nothing else of it does; a
//! malformed set is dropped whole, with a warning that names a variable,
//! and a set naming half an identity warns that it does.
use super::*;
use crate::infrastructure::persistence::project_board::git::{
    CommandScopeWarning, MAX_COMMAND_CONFIG,
};
use std::collections::BTreeMap;

/// git's command-scope configuration for `settings`, numbered from 0,
/// with `GIT_CONFIG_COUNT` given as `count`.
fn command_config(count: &str, settings: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut vars = vec![("GIT_CONFIG_COUNT".to_string(), count.to_string())];
    for (n, (key, value)) in settings.iter().enumerate() {
        vars.push((format!("GIT_CONFIG_KEY_{n}"), (*key).to_string()));
        vars.push((format!("GIT_CONFIG_VALUE_{n}"), (*value).to_string()));
    }
    vars
}

/// The variables a git built over `vars` (beside `PATH`) is started with.
fn reaching_git(repos: &Repos, vars: &[(String, String)]) -> BTreeMap<String, String> {
    let vars: Vec<(&str, &str)> = vars
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let git = shim(repos, "#!/bin/sh\nenv\n", &vars);
    git.text(&["anything"], None)
        .unwrap()
        .lines()
        .filter_map(|row| row.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// The `GIT_CONFIG_*` variables among `vars`.
fn command_scope(vars: &BTreeMap<String, String>) -> BTreeMap<&str, &str> {
    vars.iter()
        .filter(|(key, _)| key.starts_with("GIT_CONFIG"))
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
}

/// `vars` as an environment's variables.
fn os_vars(vars: Vec<(String, String)>) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect()
}

/// What `vars`' command-scope set warns of, if anything.
fn warning(vars: Vec<(OsString, OsString)>) -> Option<CommandScopeWarning> {
    GitEnvironment::allowlisted(vars)
        .command_scope_warning()
        .cloned()
}

/// The command scope git sees for `settings`, numbered from 0.
fn identity_scope(settings: &[(&str, &str)]) -> BTreeMap<String, String> {
    command_config(&settings.len().to_string(), settings)
        .into_iter()
        .collect()
}

/// [`command_scope`] owned, to compare with [`identity_scope`].
fn seen(reached: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    command_scope(reached)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// An identity set in git's command scope (`user.name` and `user.email`)
/// reaches git whole, and nothing else of the scope does: no setting
/// numbered past the count, and no other `GIT_*` variable.
#[test]
fn an_identity_in_the_command_scope_reaches_git_and_nothing_else_does() {
    let repos = Repos::new();
    let identity = [
        ("user.name", "quecto pocket"),
        ("user.email", "pocket@quecto.invalid"),
    ];
    let mut vars = command_config("2", &identity);
    let unrelated = [
        ("GIT_CONFIG_KEY_2", "core.sshCommand"),
        ("GIT_CONFIG_VALUE_2", "touch /tmp/owned"),
        ("GIT_CONFIG_PARAMETERS", "'core.hooksPath'='/tmp'"),
        ("GIT_CONFIG", "/tmp/config"),
        ("GIT_CONFIG_SYSTEM", "/tmp/config"),
        ("GIT_EXEC_PATH", "/tmp"),
        ("GIT_ASKPASS", "/tmp/askpass"),
        ("GIT_TRACE", "1"),
        ("GIT_DIR", "/tmp"),
    ];
    vars.extend(unrelated.map(|(key, value)| (key.to_string(), value.to_string())));
    assert_eq!(warning(os_vars(vars.clone())), None);
    let reached = reaching_git(&repos, &vars);
    assert_eq!(seen(&reached), identity_scope(&identity));
    for (key, _) in unrelated {
        assert!(!reached.contains_key(key), "{key} reached git");
    }
}

/// Of a set that names other keys too, only its identity settings reach
/// git, renumbered from 0, with no warning: quecto's own container scripts
/// set the identity beside a credential helper. A key matches as git
/// matches a two-part key, in any case, and passes in lower case; settings
/// that run programs, the author's and committer's own keys (the board
/// names both from `user.*`), keys that carry a credential, and keys git
/// would refuse (empty, a trailing space) are skipped, never quoted.
#[test]
fn only_the_identity_settings_of_a_set_reach_git_renumbered() {
    let repos = Repos::new();
    let helper = (
        "credential.https://github.com.helper",
        "!gh auth git-credential",
    );
    let identity = [
        ("user.name", "quecto pocket"),
        ("user.email", "pocket@quecto.invalid"),
    ];
    // The container scripts' exact shape (create.sh, exec.sh).
    let container = command_config("3", &[identity[0], identity[1], helper]);
    assert_eq!(warning(os_vars(container.clone())), None);
    assert_eq!(
        seen(&reaching_git(&repos, &container)),
        identity_scope(&identity)
    );
    let skipped = [
        helper.0,
        "core.sshCommand",
        "core.hooksPath",
        "credential.helper",
        "include.path",
        "url.https://x-access-token:ghp_TOKEN1234567890@github.com/.insteadOf",
        "author.name",
        "committer.email",
        "user.signingKey",
        "user.name ",
        " user.name",
        "user..name",
        "",
    ];
    for key in skipped {
        let vars = command_config(
            "4",
            &[
                (key, "touch /tmp/owned"),
                identity[0],
                (key, "x"),
                identity[1],
            ],
        );
        assert_eq!(warning(os_vars(vars.clone())), None, "{key:?}");
        assert_eq!(
            seen(&reaching_git(&repos, &vars)),
            identity_scope(&identity),
            "{key:?}"
        );
        let none = command_config("1", &[(key, "touch /tmp/owned")]);
        assert_eq!(command_scope(&reaching_git(&repos, &none)), BTreeMap::new());
        assert_eq!(warning(os_vars(none)), None, "{key:?}");
    }
    let shouted = command_config(
        "2",
        &[("User.Name", identity[0].1), ("USER.EMAIL", identity[1].1)],
    );
    assert_eq!(warning(os_vars(shouted.clone())), None);
    assert_eq!(
        seen(&reaching_git(&repos, &shouted)),
        identity_scope(&identity)
    );
}

/// git takes each key of the scope on its own: a set naming `user.name`
/// alone leaves `user.email` to the repository, and warns that it does;
/// so does one naming `user.email` alone.
#[test]
fn a_set_naming_half_an_identity_passes_it_and_warns() {
    let repos = Repos::new();
    for (named, unnamed) in [("user.name", "user.email"), ("user.email", "user.name")] {
        let vars = command_config("2", &[(named, "quecto pocket"), ("core.pager", "x")]);
        assert_eq!(
            seen(&reaching_git(&repos, &vars)),
            identity_scope(&[(named, "quecto pocket")])
        );
        let why = warning(os_vars(vars)).unwrap();
        assert_eq!(why, CommandScopeWarning::HalfIdentity { named, unnamed });
        assert_eq!(
            why.to_string(),
            format!(
                "git's command-scope configuration (GIT_CONFIG_COUNT) sets {named} \
                 but not {unnamed}, which comes from the repository's own"
            )
        );
    }
}

/// A key that is not UTF-8 is skipped like any other key not allowed;
/// the identity beside it still passes.
#[test]
fn a_key_that_is_not_utf8_is_skipped() {
    use std::os::unix::ffi::OsStringExt;
    let repos = Repos::new();
    let bin = repos.dir.path().join("bin");
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), path),
        ("GIT_CONFIG_COUNT".into(), "3".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            OsString::from_vec(b"user.\xffname".to_vec()),
        ),
        ("GIT_CONFIG_VALUE_0".into(), "x".into()),
        ("GIT_CONFIG_KEY_1".into(), "user.name".into()),
        ("GIT_CONFIG_VALUE_1".into(), "quecto pocket".into()),
        ("GIT_CONFIG_KEY_2".into(), "user.email".into()),
        ("GIT_CONFIG_VALUE_2".into(), "p@q.invalid".into()),
    ];
    assert_eq!(warning(vars.clone()), None);
    // The shim's git prints its environment; built here over `vars`.
    shim(&repos, "#!/bin/sh\nenv\n", &[]);
    let git = Git::new(
        &repos.alice,
        GitEnvironment::allowlisted(vars),
        person("Ada"),
    )
    .unwrap();
    let reached: BTreeMap<String, String> = git
        .text(&["anything"], None)
        .unwrap()
        .lines()
        .filter_map(|row| row.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    assert_eq!(
        seen(&reached),
        identity_scope(&[
            ("user.name", "quecto pocket"),
            ("user.email", "p@q.invalid")
        ])
    );
}

/// A command-scope set read more strictly than git reads it (a count that
/// is not plain decimal 0 to 16, a setting missing its key or its value) is
/// dropped whole, named by its variable; a set at the cap passes, and a
/// count of none passes nothing, as no set at all does.
#[test]
fn a_malformed_set_is_dropped_whole_naming_its_variable() {
    let repos = Repos::new();
    let at_cap: Vec<(&str, &str)> = [
        ("user.name", "quecto pocket"),
        ("user.email", "p@q.invalid"),
    ]
    .repeat(MAX_COMMAND_CONFIG / 2);
    let reached = reaching_git(&repos, &command_config("16", &at_cap));
    assert_eq!(seen(&reached), identity_scope(&at_cap));
    assert_eq!(warning(os_vars(command_config("16", &at_cap))), None);
    // A count of none, or an empty one (git reads it so), is no set.
    for count in ["0", ""] {
        let none = command_config(count, &[]);
        assert_eq!(command_scope(&reaching_git(&repos, &none)), BTreeMap::new());
        assert_eq!(warning(os_vars(none)), None, "{count:?}");
        let ignored = command_config(count, &[("user.name", "quecto pocket")]);
        assert_eq!(
            command_scope(&reaching_git(&repos, &ignored)),
            BTreeMap::new()
        );
        assert_eq!(warning(os_vars(ignored)), None, "{count:?}");
    }
    let past_cap = vec![("user.name", "quecto pocket"); MAX_COMMAND_CONFIG + 1];
    let one = [("user.name", "quecto pocket")];
    let count = CommandScopeWarning::Count;
    let mut cases = vec![(count.clone(), command_config("17", &past_cap))];
    for text in [
        "x",
        "-1",
        "+1",
        " 1",
        "1 ",
        "1k",
        "0x1",
        "١",
        "99999999999999999999999",
    ] {
        cases.push((count.clone(), command_config(text, &one)));
    }
    let missing = |variable: &str| CommandScopeWarning::Missing {
        variable: variable.into(),
    };
    let mut no_value = command_config("1", &one);
    no_value.retain(|(key, _)| key != "GIT_CONFIG_VALUE_0");
    let mut no_key = command_config("1", &one);
    no_key.retain(|(key, _)| key != "GIT_CONFIG_KEY_0");
    // A missing value of a setting that would be skipped still drops the
    // set whole.
    let mut skipped_no_value = command_config("2", &[one[0], ("core.pager", "less")]);
    skipped_no_value.retain(|(key, _)| key != "GIT_CONFIG_VALUE_1");
    cases.extend([
        (missing("GIT_CONFIG_VALUE_0"), no_value),
        (missing("GIT_CONFIG_KEY_0"), no_key),
        (missing("GIT_CONFIG_KEY_1"), command_config("2", &one)),
        (missing("GIT_CONFIG_VALUE_1"), skipped_no_value),
    ]);
    for (why, vars) in cases {
        let reached = reaching_git(&repos, &vars);
        assert_eq!(command_scope(&reached), BTreeMap::new(), "{vars:?}");
        assert_eq!(warning(os_vars(vars.clone())), Some(why), "{vars:?}");
    }
}

/// The reason a set was dropped names a variable, in words fixed by the
/// board: never a key or a value the caller set, so a credential in a key
/// (a `url.<base>.insteadOf` with a token in it) is never quoted.
#[test]
fn a_malformed_sets_reason_quotes_no_key_or_value() {
    let token = "ghp_TOKEN1234567890";
    let key = format!("url.https://x-access-token:{token}@github.com/.insteadOf");
    let mut vars = command_config(
        "2",
        &[(key.as_str(), "https://github.com/"), ("user.name", token)],
    );
    vars.retain(|(name, _)| name != "GIT_CONFIG_VALUE_1");
    let why = warning(os_vars(vars)).unwrap();
    assert_eq!(
        why.to_string(),
        "ignoring git's command-scope configuration (GIT_CONFIG_COUNT): \
         GIT_CONFIG_VALUE_1 is not set"
    );
    let vars = command_config(token, &[(key.as_str(), token)]);
    assert_eq!(
        warning(os_vars(vars)).unwrap().to_string(),
        "ignoring git's command-scope configuration (GIT_CONFIG_COUNT): \
         GIT_CONFIG_COUNT is not plain decimal 0 to 16"
    );
}
