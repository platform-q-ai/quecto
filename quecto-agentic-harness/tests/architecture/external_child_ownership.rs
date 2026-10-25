//! Exercise the real ownership rules in isolated workspaces, without changing
//! process-global cwd or compiling fixture imports. Only canonical modules are
//! involved; raw identifiers, include! and path aliases are separate concerns.
use std::{fs, path::Path, process::Command};

/// Catches a checker that hangs; it is not a performance budget (#2736).
/// Each checker syn-parses the whole copied crate (about 600k lines, in a
/// debug build). That takes 10-20 s alone and well over 30 s at load
/// average ~20, which is how a 30 s guard came to fail with no regression.
/// Four minutes is far above any healthy run on a loaded machine, and it
/// fires before nextest's CI profile kills the test at five minutes
/// (`.config/nextest.toml`), so a hang fails with this guard's message.
const HANG_GUARD_SECS: u32 = 240;

struct Fixture(tempfile::TempDir);

impl Fixture {
    fn new() -> Self {
        fn copy(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() {
                    copy(&entry.path(), &to.join(entry.file_name()));
                } else {
                    assert!(kind.is_file(), "fixture sources must be regular files");
                    fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
                }
            }
        }
        let fixture = Self(tempfile::tempdir().unwrap());
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for krate in ["quecto-agentic-harness", "quecto-tui"] {
            copy(
                &workspace.join(krate).join("src"),
                &fixture.0.path().join(krate).join("src"),
            );
        }
        fixture
    }

    fn root(&self) -> std::path::PathBuf {
        self.0.path().join("quecto-agentic-harness")
    }

    fn append(&self, owner: &str, source: &str) {
        let path = self.root().join(owner);
        let original = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{original}\n{source}\n")).unwrap();
    }

    fn child(&self, owner: &str, name: &str, source: &str) {
        let directory = self.root().join(owner.trim_end_matches(".rs"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{name}.rs")), source).unwrap();
    }

    fn check(&self, checker: &str, diagnostic: Option<&str>) -> String {
        let started = std::time::Instant::now();
        let output = Command::new("timeout")
            .args(["-s", "KILL", &HANG_GUARD_SECS.to_string()])
            .arg(std::env::current_exe().unwrap())
            .args([checker, "--exact", "--nocapture"])
            .current_dir(self.root())
            .output()
            .unwrap();
        let elapsed = started.elapsed();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!(
            "fixture {}: {checker} ({elapsed:.1?})\n{text}",
            self.0.path().display()
        );
        // Allowlist of the two outcomes a finished checker can have: 0 when
        // it passed, 101 when it failed. Anything else means it crashed or
        // ran into the hang guard (its SIGKILL ends GNU timeout too, so the
        // status carries the signal and no exit code).
        assert!(
            matches!(output.status.code(), Some(0 | 101)),
            "{checker} ended with {} after {elapsed:.1?}: a checker that ends \
             any other way crashed or exceeded the {HANG_GUARD_SECS} s hang \
             guard\n{text}",
            output.status
        );
        assert!(text.contains("running 1 test"), "{checker}: {text}");
        match diagnostic {
            Some(needle) => {
                assert_eq!(output.status.code(), Some(101), "{checker}: {text}");
                assert!(
                    needle.split('|').any(|message| text.contains(message)),
                    "{checker}: {text}"
                );
                assert!(text.contains("1 failed"), "{checker}: {text}");
            }
            None => {
                assert!(output.status.success(), "{checker}: {text}");
                assert!(text.contains("1 passed"), "{checker}: {text}");
            }
        }
        text
    }
}

// Each policy has independently runnable controls and RED, so a missed child
// never prevents evidence that direct violations and legitimate imports work.
macro_rules! policy {
    ($name:ident, $checker:expr, $owner:expr, $bad:expr, $good:expr, $diagnostic:expr) => {
        mod $name {
            use super::Fixture;
            #[test]
            fn direct_owner_is_rejected() {
                let f = Fixture::new();
                f.append($owner, $bad);
                f.check($checker, Some($diagnostic));
            }
            #[test]
            fn own_capability_sibling_and_tests_are_accepted() {
                let f = Fixture::new();
                f.append(
                    $owner,
                    &format!(
                        "{}\n#[cfg(test)] mod regression_tests;\n\
                         #[cfg(test)] mod inline_tests {{ {} }}",
                        $good, $bad
                    ),
                );
                f.child($owner, "regression_tests", $bad);
                // A shared-role sibling is not owned by this capability.
                let sibling = Path::new($owner).parent().unwrap().join("unrelated.rs");
                fs::write(f.root().join(&sibling), $bad).unwrap();
                f.append(
                    sibling.parent().unwrap().join("mod.rs").to_str().unwrap(),
                    "mod unrelated;",
                );
                f.check($checker, None);
            }
            #[test]
            fn ordinary_external_child_is_rejected() {
                let f = Fixture::new();
                f.append($owner, "mod regression_child;");
                f.child($owner, "regression_child", $bad);
                f.check($checker, Some($diagnostic));
            }
            use std::{fs, path::Path};
        }
    };
}

policy!(
    teardown_capability,
    "subagent_teardown_interface_only_parses_maps_and_presents",
    "src/interface/uds/mapping/subagent_teardown_mapping.rs",
    "use crate::application::sessions::ports::SessionStore;",
    "use crate::application::subagents::dto::Report;\nuse crate::interface::uds::presenters::subagent_teardown_presenter::Report;",
    "depends outward or on forbidden vocabulary"
);
policy!(
    sessions_capability,
    "sessions_interface_only_parses_maps_and_presents",
    "src/interface/uds/controllers/read_history_controller.rs",
    "use crate::application::subagents::dto::Report;",
    "use crate::application::sessions::dto::Report;\nuse crate::interface::uds::controllers::sessions_controller::Report;",
    "depends outward or on forbidden vocabulary"
);
policy!(
    catalogue_capability,
    "catalogue_interface_only_parses_maps_and_presents",
    "src/interface/uds/mapping/persist_scope.rs",
    "use crate::application::catalogue::ports::Store;",
    "use crate::application::catalogue::dto::Report;\nuse crate::interface::uds::presenters::model_presenter::Report;",
    "depends outward or on forbidden vocabulary"
);
policy!(
    teardown_behavior,
    "teardown_layers::teardown_interface_only_parses_maps_and_presents",
    "src/interface/uds/mapping/subagent_teardown_mapping.rs",
    "fn forbidden() { supervisor.request_termination(); }",
    "fn map_only() {}",
    "does more than parse/map/present"
);
policy!(
    teardown_pid,
    "teardown_authority::teardown_paths_carry_no_pid",
    "src/interface/uds/mapping/subagent_teardown_mapping.rs",
    "fn forbidden(pid: u32) {}",
    "fn map_only() {}",
    "names a pid on a teardown path|is a teardown module not covered by the no-pid ratchet"
);
policy!(
    sessions_edge,
    "sessions_epic_close_retirement::interface_session_modules_parse_map_and_present_only",
    "src/interface/uds/controllers/read_history_controller.rs",
    "use crate::application::sessions::ports::SessionStore;",
    "use crate::application::sessions::dto::Report;",
    "imports beyond the capability's DTOs/use cases"
);

// Passing characterization, not the policy RED: a real production offender
// must be reported independently of the two inherited test-file offenders.
#[test]
fn sessions_constructor_still_rejects_another_production_file() {
    let f = Fixture::new();
    f.append(
        "src/application/sessions/services/history_paging.rs",
        "fn forbidden() { ActiveSessionState::new(identity, store); }",
    );
    f.check(
        "sessions_capability::sessions_graph_nodes_are_constructed_only_in_composition",
        Some("src/application/sessions/services/history_paging.rs"),
    );
}

// Passing baseline characterization, not a policy RED: the exact raw-key
// inventory must reject an ordinary production sibling even with the approved
// active-session cfg(test) child present.
#[test]
fn sessions_raw_key_still_rejects_another_production_file() {
    let f = Fixture::new();
    f.append(
        "src/application/sessions/services/history_paging.rs",
        "fn forbidden() { SessionIdentity::from_persisted_key(key); }",
    );
    f.check(
        "sessions_epic_close_retirement::raw_key_conversion_and_startup_key_sites_are_exact",
        Some("src/application/sessions/services/history_paging.rs"),
    );
}

#[test]
fn sessions_retention_still_rejects_another_production_file() {
    let f = Fixture::new();
    f.append(
        "src/application/sessions/services/history_paging.rs",
        "use crate::application::sessions::ports::ContextSpillStore;",
    );
    f.check(
        "sessions_capability::retained_context_has_exactly_one_owner_per_role",
        Some("src/application/sessions/services/history_paging.rs"),
    );
}

// #2730: the sessions exceptions #2650 admits are two exact files, each for
// exact roles: services/active_session/tests.rs constructs
// ActiveSessionState and converts raw keys; services/conversation_ledger/
// tests.rs holds the retention port. Each role's matrix pins that only the
// approved child is admitted, so a broader exemption (a glob over
// `*/active_session/tests.rs`, any `tests.rs`, a prefix or suffix match, a
// whole folder, or the other approved file's role) fails here even though
// the clean-tree inventories would still pass.
const SESSIONS_SERVICES: &str = "src/application/sessions/services";

/// Plants `bad` in a mounted cfg(test) child at `target`: appended when the
/// file exists, otherwise created and declared by its parent module (which
/// is itself created and declared when missing).
fn plant(f: &Fixture, target: &str, bad: &str) {
    let path = f.root().join(target);
    if path.is_file() {
        f.append(target, bad);
        return;
    }
    let parent = Path::new(target).parent().unwrap();
    let module = Path::new(target).file_stem().unwrap().to_str().unwrap();
    let owner = parent.with_extension("rs");
    let declaration = format!("#[cfg(test)] mod {module};");
    if f.root().join(&owner).is_file() {
        f.append(owner.to_str().unwrap(), &declaration);
    } else {
        let folder = parent.parent().unwrap();
        let name = parent.file_name().unwrap().to_str().unwrap();
        f.append(
            folder.join("mod.rs").to_str().unwrap(),
            &format!("mod {name};"),
        );
        fs::write(f.root().join(&owner), format!("{declaration}\n")).unwrap();
    }
    fs::create_dir_all(f.root().join(parent)).unwrap();
    fs::write(&path, bad).unwrap();
    assert!(path.is_file(), "planted {target}");
}

/// The role's checker must fail with its own diagnostic and name `target`.
fn rejects(checker: &str, bad: &str, diagnostic: &str, target: &str) {
    let f = Fixture::new();
    plant(&f, target, bad);
    let text = f.check(checker, Some(diagnostic));
    assert!(
        text.contains(target),
        "{checker} must name {target}: {text}"
    );
}

macro_rules! exception_role {
    ($name:ident, $checker:expr, $allowed:expr, $opposite:expr, $bad:expr, $diagnostic:expr) => {
        mod $name {
            use super::{Fixture, SESSIONS_SERVICES, rejects};
            fn approved() -> String {
                format!("{SESSIONS_SERVICES}/{}/tests.rs", $allowed)
            }
            fn rejected(target: &str) {
                rejects($checker, $bad, $diagnostic, target);
            }
            #[test]
            fn approved_child_is_accepted() {
                let f = Fixture::new();
                f.append(&approved(), $bad);
                f.check($checker, None);
            }
            // A production owner's inline cfg(test) items are not production
            // code, so they need no policy role.
            #[test]
            fn production_owner_inline_tests_are_accepted() {
                let f = Fixture::new();
                f.append(
                    &format!("{SESSIONS_SERVICES}/history_paging.rs"),
                    &format!("#[cfg(test)] mod review_tests {{ {} }}", $bad),
                );
                f.check($checker, None);
            }
            #[test]
            fn typo_child_is_rejected() {
                rejected(&format!("{SESSIONS_SERVICES}/{}/test.rs", $allowed));
            }
            #[test]
            fn prefixed_child_is_rejected() {
                rejected(&format!("{SESSIONS_SERVICES}/{}/xtests.rs", $allowed));
            }
            #[test]
            fn suffixed_child_is_rejected() {
                rejected(&format!("{SESSIONS_SERVICES}/{}/tests_extra.rs", $allowed));
            }
            #[test]
            fn same_basename_in_another_service_is_rejected() {
                rejected(&format!("{SESSIONS_SERVICES}/history_paging/tests.rs"));
            }
            #[test]
            fn same_child_under_another_role_folder_is_rejected() {
                rejected(&format!(
                    "src/application/sessions/ports/{}/tests.rs",
                    $allowed
                ));
            }
            #[test]
            fn production_sibling_is_rejected() {
                rejected(&format!("{SESSIONS_SERVICES}/session_home.rs"));
            }
            // The other approved file is exact for its own roles only.
            #[test]
            fn opposite_approved_child_is_rejected() {
                rejected(&format!("{SESSIONS_SERVICES}/{}/tests.rs", $opposite));
            }
            // The exception names one file: a moved or renamed child fails
            // rather than silently leaving the exception unused.
            #[test]
            fn missing_approved_child_fails() {
                let f = Fixture::new();
                std::fs::remove_file(f.root().join(approved())).unwrap();
                let text = f.check($checker, Some("assertion `left == right` failed"));
                assert!(text.contains(&approved()), "{text}");
            }
            #[test]
            fn malformed_approved_child_fails() {
                let f = Fixture::new();
                f.append(&approved(), "fn malformed(");
                f.check($checker, Some("parse module file"));
            }
        }
    };
}

exception_role!(
    sessions_constructor_exception,
    "sessions_capability::sessions_graph_nodes_are_constructed_only_in_composition",
    "active_session",
    "conversation_ledger",
    "fn review_bad() { ActiveSessionState::new(identity, store); }",
    "ActiveSessionState::new( may appear only in"
);
exception_role!(
    sessions_raw_key_exception,
    "sessions_epic_close_retirement::raw_key_conversion_and_startup_key_sites_are_exact",
    "active_session",
    "conversation_ledger",
    "fn review_bad() { SessionIdentity::from_persisted_key(key); }",
    "the raw-key conversion sites are exact"
);
exception_role!(
    sessions_retention_exception,
    "sessions_capability::retained_context_has_exactly_one_owner_per_role",
    "conversation_ledger",
    "active_session",
    "use crate::application::sessions::ports::ContextSpillStore;",
    "the retention port's holders are exact"
);

// The constructor exception covers ActiveSessionState only: the approved
// child gains no other composed constructor.
#[test]
fn sessions_approved_child_gains_no_other_constructor() {
    rejects(
        "sessions_capability::sessions_graph_nodes_are_constructed_only_in_composition",
        "fn review_bad() { ListSessions::new(store); }",
        "ListSessions::new( may appear only in",
        "src/application/sessions/services/active_session/tests.rs",
    );
}
