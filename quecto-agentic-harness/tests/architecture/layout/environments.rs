//! Binding #2362 move-map acceptance; no imports of future production modules.
use super::{Entry, Kind, domain, scan};
use std::path::PathBuf;

const DESTINATIONS: &[&str] = &[
    "entities/environment_journal.rs",
    "services/environment_listing.rs",
    "services/environment_listing_tests.rs",
    "entities/environment_registry.rs",
    "entities/environment_registry/record_lookup.rs",
    "entities/environment_registry/record_lookup_tests.rs",
    "entities/environment_registry/record_residue.rs",
    "entities/environment_registry/record_residue_tests.rs",
    "entities/environment_registry_inspect.rs",
    "entities/environment_registry_removal.rs",
    "entities/environment_registry_removal_tests.rs",
    "entities/environment_registry_slice2_tests.rs",
    "entities/environment_registry_slice3_tests.rs",
    "entities/environment_registry_tests.rs",
    "services/environment_retention.rs",
    "services/environment_retention_owner_end_tests.rs",
    "services/environment_retention_tests.rs",
];
const RETIRED: &[&str] = &[
    "environment_journal.rs",
    "environment_listing.rs",
    "environment_registry.rs",
    "environment_registry_inspect.rs",
    "environment_registry_removal.rs",
    "environment_retention.rs",
    "environment_registry",
];
const ROLES: &[&str] = &["entities", "services"];

fn capability() -> PathBuf {
    domain().join("environments")
}

#[test]
fn all_seventeen_mapped_destinations_exist() {
    assert_eq!(DESTINATIONS.len(), 17, "binding move-map size");
    let observed: Vec<_> = DESTINATIONS
        .iter()
        .map(|path| (*path, capability().join(path).is_file()))
        .collect();
    assert!(
        observed.iter().all(|(_, present)| *present),
        "all 17 destination files must exist (path, present): {observed:?}"
    );
}

#[test]
fn six_flat_modules_and_old_registry_directory_are_retired() {
    let remaining: Vec<_> = RETIRED
        .iter()
        .filter(|path| domain().join(path).exists())
        .collect();
    assert!(
        remaining.is_empty(),
        "old paths still present: {remaining:?}"
    );
}

fn direct_production(entry: &Entry) -> bool {
    match entry.0.as_str() {
        "mod.rs" | "mod_tests.rs" => false,
        name => match name.strip_suffix("_tests.rs") {
            Some(_) => false,
            None => super::regular_rust(entry),
        },
    }
}

#[test]
fn declared_roles_exist_with_two_direct_production_files_each() {
    let counts: Vec<_> = ROLES
        .iter()
        .map(|role| {
            let path = capability().join(role);
            let count = match scan(&path) {
                Ok(entries) => Some(
                    entries
                        .iter()
                        .filter(|entry| direct_production(entry))
                        .count(),
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("cannot inspect {}: {error}", path.display()),
            };
            (*role, count)
        })
        .collect();
    assert!(
        counts.iter().all(|(_, count)| matches!(count, Some(2..))),
        "required roles need >=2 direct production files, excluding mod/test files: {counts:?}"
    );
}

fn root_entries() -> Vec<Entry> {
    let path = capability();
    assert!(
        path.is_dir(),
        "required capability directory absent: {}",
        path.display()
    );
    scan(&path).expect("capability must be a readable real directory")
}

#[test]
fn capability_root_contains_only_module_wiring_and_declared_roles() {
    let entries = root_entries();
    assert!(
        entries.iter().all(|entry| match entry.1 {
            Kind::File => matches!(entry.0.as_str(), "mod.rs" | "mod_tests.rs"),
            Kind::Directory => ROLES.contains(&entry.0.as_str()),
        }),
        "capability root allows only mod.rs/mod_tests.rs and entities/services"
    );
}

#[test]
fn all_capability_files_live_in_declared_roles() {
    let entries = root_entries();
    for entry in entries {
        match entry.1 {
            Kind::File if matches!(entry.0.as_str(), "mod.rs" | "mod_tests.rs") => {}
            Kind::Directory if ROLES.contains(&entry.0.as_str()) => {
                inspect_role_tree(&capability().join(entry.0));
            }
            _ => panic!("file or subtree outside declared roles: {}", entry.0),
        }
    }
}

fn inspect_role_tree(path: &std::path::Path) {
    for entry in scan(path).expect("role descendants must be readable real files/directories") {
        match entry.1 {
            Kind::File => {}
            Kind::Directory => inspect_role_tree(&path.join(entry.0)),
        }
    }
}
