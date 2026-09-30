use super::*;
const SESSIONS: &str = "domain/sessions";
const ENTITIES: &str = "domain/sessions/entities";
struct Tree(tempfile::TempDir);
impl Tree {
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn at(&self, name: impl AsRef<Path>) -> std::path::PathBuf {
        self.path().join(name)
    }
    fn mkdir(&self, name: &str) {
        std::fs::create_dir_all(self.path().join(name)).unwrap();
    }
    fn write(&self, name: &str) {
        std::fs::write(self.path().join(name), "").unwrap();
    }
}
fn fixture(dirs: &[&str], files: &[&str]) -> Tree {
    let tree = Tree(tempfile::tempdir().unwrap());
    for dir in dirs {
        tree.mkdir(dir);
    }
    for file in files {
        tree.mkdir(Path::new(file).parent().unwrap().to_str().unwrap());
        tree.write(file);
    }
    tree
}
fn budget(path: &str, expected: usize) -> Budget<'_> {
    Budget(path, expected)
}
fn capability<'a>(path: &'a str, allowed_roles: &'a [&'a str]) -> Capability<'a> {
    Capability {
        path,
        allowed_roles,
    }
}
fn flat(root: &Path, path: &str, expected: usize) -> Vec<Violation> {
    validate(root, &[budget(path, expected)], &[])
}
fn shape(root: &Path, path: &str, roles: &[&str]) -> Vec<Violation> {
    validate(root, &[], &[capability(path, roles)])
}
fn entities(root: &Path) -> Vec<Violation> {
    shape(root, SESSIONS, &["entities"])
}
fn mismatch(path: &str, actual: usize, expected: usize) -> Violation {
    Violation::BudgetMismatch {
        path: path.into(),
        actual,
        expected,
    }
}
fn inspection(failures: &[Violation], path: Option<&str>, reason: &str) {
    assert!(
        matches!(failures, [Violation::Inspection { path: actual, reason: why }]
        if path.is_none_or(|path| path == actual) && why.contains(reason)),
        "{failures:?}"
    );
}
fn fragments(failures: &[Violation], required: &[&str]) {
    let message = format!("{failures:?}");
    for text in required {
        assert!(message.contains(text), "{message}");
    }
}
fn populate_transitional(root: &Tree) {
    for row in PLACEMENTS
        .iter()
        .filter(|row| row.classification == Classification::Transitional)
    {
        root.mkdir(&format!("{}/{}", row.layer, row.name));
    }
}
#[test]
fn exact_budget_counts_and_count_neutral_changes() {
    let root = fixture(
        &["domain/nested.rs"],
        &[
            "domain/mod.rs",
            "domain/support_test.rs",
            "domain/helper.rs",
            "domain/helper_tests.rs",
            "domain/README",
            "domain/helper.rs.bak",
            "domain/nested.rs/deep.rs",
        ],
    );
    assert!(flat(root.path(), "domain", 3).is_empty());
    assert_eq!(
        flat(root.path(), "domain", 2),
        vec![mismatch("domain", 3, 2)]
    );
    let root = fixture(&[], &["domain/old.rs"]);
    assert!(flat(root.path(), "domain", 1).is_empty());
    assert_eq!(flat(root.path(), "domain", 2).len(), 1);
    std::fs::rename(root.at("domain/old.rs"), root.at("domain/new.rs")).unwrap();
    assert!(flat(root.path(), "domain", 1).is_empty());
    std::fs::write(root.at("domain/new.rs"), "replacement").unwrap();
    assert!(flat(root.path(), "domain", 1).is_empty());
    root.write("domain/extra.rs");
    assert_eq!(
        flat(root.path(), "domain", 1),
        vec![mismatch("domain", 2, 1)]
    );
}
#[test]
fn migrated_shape_allowlists_sparse_roots_and_stray_files() {
    for (path, roles) in [
        (SESSIONS, &["entities", "events", "errors"][..]),
        ("application/sessions", &["use_cases", "ports", "dto"][..]),
    ] {
        let root = fixture(&[path], &[]);
        assert!(shape(root.path(), path, roles).is_empty());
        for name in ["mod.rs", "mod_tests.rs"] {
            root.write(&format!("{path}/{name}"));
        }
        assert!(shape(root.path(), path, roles).is_empty());
        root.write(&format!("{path}/stray.rs"));
        assert_eq!(
            shape(root.path(), path, roles),
            vec![Violation::StrayEntry(format!("{path}/stray.rs"))]
        );
        std::fs::remove_file(root.at(format!("{path}/stray.rs"))).unwrap();
        root.mkdir(&format!("{path}/handles"));
        assert_eq!(
            shape(root.path(), path, roles),
            vec![Violation::UndeclaredRole(format!("{path}/handles"))]
        );
    }
    let root = fixture(
        &[],
        &[
            "application/sessions/use_cases/one.rs",
            "application/sessions/ports/one.rs",
            "application/sessions/dto/one.rs",
        ],
    );
    let roles = &["use_cases", "ports", "dto"];
    assert!(shape(root.path(), "application/sessions", roles).is_empty());
    root.write("application/sessions/stray.rs");
    assert_eq!(
        shape(root.path(), "application/sessions", roles),
        vec![Violation::StrayEntry(
            "application/sessions/stray.rs".into()
        )]
    );
    let root = fixture(&[], &["domain/sessions/entities/one.rs"]);
    assert!(shape(root.path(), SESSIONS, &["entities", "events"]).is_empty());
    assert_eq!(
        shape(root.path(), SESSIONS, &["events"]),
        vec![Violation::UndeclaredRole(ENTITIES.into())]
    );
    root.write("domain/sessions/readme.txt");
    let failures = shape(root.path(), SESSIONS, &["events"]);
    assert_eq!(failures.len(), 2);
    assert!(
        failures
            .iter()
            .all(|v| matches!(v, Violation::UndeclaredRole(..) | Violation::StrayEntry(..)))
    );
}
#[test]
fn all_role_occupancy_cases_include_nested_and_test_only_rust() {
    for (file, populated) in [
        (None, false),
        (Some("README.md"), false),
        (Some("nested/deep.rs"), true),
        (Some("one.rs"), true),
        (Some("tests_tests.rs"), true),
        (Some("nested/only_tests.rs"), true),
    ] {
        let root = fixture(&[ENTITIES], &[]);
        if let Some(file) = file {
            root.mkdir(&format!(
                "{ENTITIES}/{}",
                Path::new(file).parent().unwrap().display()
            ));
            root.write(&format!("{ENTITIES}/{file}"));
        }
        let failures = entities(root.path());
        if populated {
            assert!(failures.is_empty(), "{file:?}");
        } else {
            assert_eq!(
                failures,
                vec![Violation::EmptyRole(ENTITIES.into())],
                "{file:?}"
            );
        }
    }
}
#[test]
fn missing_substituted_and_restored_roots_and_injected_denials() {
    let root = fixture(&[], &[]);
    inspection(&flat(root.path(), "domain", 0), None, "");
    inspection(&shape(root.path(), SESSIONS, &[]), None, "");
    root.write("domain");
    inspection(&flat(root.path(), "domain", 0), None, "");
    std::fs::remove_file(root.at("domain")).unwrap();
    root.mkdir("domain");
    assert!(flat(root.path(), "domain", 0).is_empty());
    root.write(SESSIONS);
    inspection(&shape(root.path(), SESSIONS, &[]), None, "");
    root.mkdir("domain/environment_registry");
    let failures = placements(root.path(), PLACEMENTS);
    assert_eq!(failures.len(), 4);
    assert!(
        failures
            .iter()
            .all(|v| matches!(v, Violation::Inspection { .. }))
    );
    root.write("application");
    assert_eq!(placements(root.path(), PLACEMENTS).len(), 4);
    for (path, reason) in [
        ("domain", "entry metadata denied"),
        (ENTITIES, "role read denied"),
    ] {
        let root = fixture(&[ENTITIES], &["domain/sessions/entities/one.rs"]);
        let failures = validate_all(
            root.path(),
            &[budget("domain", 0)],
            &[capability(SESSIONS, &["entities"])],
            None,
            &|entry| {
                if entry.ends_with(path) {
                    Err(io::Error::new(io::ErrorKind::PermissionDenied, reason))
                } else {
                    scan(entry)
                }
            },
        );
        inspection(&failures, Some(path), reason);
    }
    for substituted in [false, true] {
        let root = fixture(
            &["application", "interface", "infrastructure", "composition"],
            &[],
        );
        if substituted {
            root.write("domain");
        }
        inspection(
            &validate_all(root.path(), &[budget("domain", 0)], &[], Some(&[]), &scan),
            Some("domain"),
            "",
        );
    }
}
#[test]
fn placement_future_targets_unknowns_and_stale_transitional_rows() {
    for (known, unknown) in [
        ("application/sessions", "application/helpers"),
        (SESSIONS, "domain/unknown"),
    ] {
        let root = fixture(LAYERS, &[]);
        populate_transitional(&root);
        assert!(placements(root.path(), PLACEMENTS).is_empty());
        root.mkdir(known);
        root.write("application/legacy.rs");
        assert!(placements(root.path(), PLACEMENTS).is_empty());
        root.mkdir(unknown);
        assert_eq!(
            placements(root.path(), PLACEMENTS),
            vec![Violation::UnlistedPlacement(unknown.into())]
        );
    }
    let root = fixture(LAYERS, &[]);
    let rows = [Placement {
        layer: "domain",
        name: "removed",
        _citation: WIKI,
        classification: Classification::Transitional,
    }];
    let failures = placements(root.path(), &rows);
    assert_eq!(failures.len(), 1);
    fragments(&failures, &["remove domain/removed from PLACEMENTS", WIKI]);
}
#[test]
fn diagnostics_prescribe_each_actual_repair_and_budget_direction() {
    for (directory, file, required) in [
        (
            SESSIONS,
            Some("domain/sessions/stray.rs"),
            "move domain/sessions/stray.rs into a declared role directory",
        ),
        (
            "domain/sessions/handles",
            None,
            "add handles to domain/sessions allowed_roles in MIGRATED",
        ),
        (
            ENTITIES,
            None,
            "put a .rs file anywhere beneath domain/sessions/entities or remove the unused role directory",
        ),
    ] {
        let files: Vec<_> = file.into_iter().collect();
        let root = fixture(&[directory], &files);
        let failures = entities(root.path());
        assert_eq!(failures.len(), 1);
        fragments(&failures, &[required, WIKI]);
    }
    let root = fixture(LAYERS, &[]);
    let failures = flat(root.path(), "domain/missing", 0);
    inspection(&failures, None, "");
    fragments(&failures, &["restore domain/missing", WIKI]);
    for (failure, required) in [
        (
            mismatch("domain", 2, 1),
            &["BUDGETS", "capability", WIKI][..],
        ),
        (
            mismatch("domain", 1, 2),
            &["actual 1, expected 2", "lower domain to 1 in BUDGETS"][..],
        ),
        (
            Violation::UnlistedPlacement("domain/widgets".into()),
            &["add a placement row with this capability's wiki section"][..],
        ),
        (
            Violation::EmptyRole(ENTITIES.into()),
            &["put a .rs file anywhere beneath domain/sessions/entities"][..],
        ),
    ] {
        fragments(&[failure], required);
    }
}
#[test]
fn relative_root_is_canonicalized_before_scanner_port() {
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&cwd).unwrap();
    std::fs::create_dir(root.path().join("domain")).unwrap();
    let failures = validate_all(
        root.path().strip_prefix(&cwd).unwrap(),
        &[budget("domain", 0)],
        &[],
        None,
        &|path| {
            assert!(
                path.is_absolute(),
                "canonical root boundary passes absolute paths"
            );
            scan(path)
        },
    );
    assert!(failures.is_empty());
}
#[cfg(unix)]
#[test]
fn symlink_ancestors_descendants_and_caller_root_aliases() {
    use std::os::unix::fs::symlink;
    for (name, role) in [
        ("domain/linked.rs", false),
        ("domain/sub", false),
        ("domain/sessions/mod.rs", true),
        (ENTITIES, true),
        ("domain/sessions/entities/linked.rs", true),
        ("domain/sessions/entities/nested/link.rs", true),
    ] {
        let root = fixture(
            &[SESSIONS, "domain/sessions/entities/nested"],
            &["outside.rs", "domain/sessions/entities/one.rs"],
        );
        if name == ENTITIES {
            std::fs::remove_dir_all(root.at(ENTITIES)).unwrap();
        }
        symlink(root.at("outside.rs"), root.at(name)).unwrap();
        let failures = if role {
            shape(root.path(), SESSIONS, &["entities"])
        } else {
            flat(root.path(), "domain", 0)
        };
        inspection(
            &failures,
            None,
            Path::new(name).file_name().unwrap().to_str().unwrap(),
        );
    }
    let root = fixture(&[], &[]);
    let outside = fixture(&["swarm", "domain"], &[]);
    symlink(outside.path(), root.at("domain")).unwrap();
    inspection(&flat(root.path(), "domain/swarm", 0), None, "");
    inspection(&shape(root.path(), "domain/swarm", &[]), None, "");
    symlink(outside.path(), root.at("linked_root")).unwrap();
    assert!(flat(&root.at("linked_root"), "domain", 0).is_empty());
    let real = fixture(LAYERS, &[]);
    real.mkdir(SESSIONS);
    populate_transitional(&real);
    symlink(real.path(), root.at("alias")).unwrap();
    assert!(flat(&root.at("alias"), "domain", 0).is_empty());
    assert!(
        validate_all(
            &root.at("alias"),
            &[budget("domain", 0)],
            &[],
            Some(PLACEMENTS),
            &scan
        )
        .is_empty()
    );
    symlink(outside.path(), real.at("domain/link")).unwrap();
    inspection(&placements(real.path(), PLACEMENTS), Some("domain"), "link");
}
#[cfg(unix)]
#[test]
fn non_utf8_fifo_and_real_nonroot_permission_fail_closed() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    let root = fixture(&["domain"], &[]);
    let bad_name = std::ffi::OsString::from_vec(vec![0xff]);
    std::fs::write(root.at("domain").join(&bad_name), "").unwrap();
    inspection(&flat(root.path(), "domain", 0), None, "non-UTF-8");
    std::fs::remove_file(root.at("domain").join(bad_name)).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.at("domain/special"))
            .status()
            .unwrap()
            .success()
    );
    inspection(&flat(root.path(), "domain", 0), None, "non-regular");
    let root = fixture(&[], &["domain/sessions/entities/one.rs"]);
    let file = root.at("domain/sessions/entities/one.rs");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(
        std::fs::File::open(&file).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied,
        "effective nonroot required"
    );
    let failures = entities(root.path());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    inspection(&failures, None, "one.rs");
}
