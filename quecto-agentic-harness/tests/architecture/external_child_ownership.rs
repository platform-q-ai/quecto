//! Exercise the real ownership rules in isolated workspaces, without changing
//! process-global cwd or compiling fixture imports. Only canonical modules are
//! involved; raw identifiers, include! and path aliases are separate concerns.
use std::{fs, path::Path, process::Command};

/// Catches a checker that hangs; it is not a performance budget (#2736).
/// Each checker syn-parses the whole copied crate (about 600k lines, in a
/// debug build). That takes 10-20 s alone and well over 30 s at load
/// average ~20, which is how a 30 s guard came to fail with no regression.
/// Five minutes is far above any healthy run on a loaded machine and still
/// stops a real hang well before CI's job timeout.
const HANG_GUARD_SECS: u32 = 300;

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

    fn check(&self, checker: &str, diagnostic: Option<&str>) {
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
