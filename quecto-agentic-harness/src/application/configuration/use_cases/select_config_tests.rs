use super::*;
use crate::application::configuration::dto::OverlayOrigin;
use crate::application::configuration::ports::{MainWorktreeLocator, WorktreeLink};
use crate::application::configuration::use_cases::fakes::MemoryStore;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A locator answering one link for every directory, and recording which
/// directories it was asked about.
struct FixedLocator {
    link: WorktreeLink,
    asked: Mutex<Vec<PathBuf>>,
}

impl MainWorktreeLocator for FixedLocator {
    fn locate(&self, directory: &Path) -> WorktreeLink {
        self.asked.lock().unwrap().push(directory.to_path_buf());
        self.link.clone()
    }
}

fn locator(link: WorktreeLink) -> Arc<FixedLocator> {
    Arc::new(FixedLocator {
        link,
        asked: Mutex::new(Vec::new()),
    })
}

fn select() -> SelectConfig {
    SelectConfig::new(MemoryStore::with(&[]), locator(WorktreeLink::NotLinked))
}

fn linked() -> WorktreeLink {
    WorktreeLink::Linked {
        main_worktree: PathBuf::from("/main"),
    }
}

const OWN: &str = "/work/.quecto/config.json";
const MAIN: &str = "/main/.quecto/config.json";

fn layered(overlay: &str, origin: OverlayOrigin) -> ConfigSelection {
    ConfigSelection::Layered(ConfigLayers {
        global: PathBuf::from("/home/u/.quecto/config.json"),
        overlay: Some(PathBuf::from(overlay)),
        legacy_local: Some(PathBuf::from("/work/config.json")),
        overlay_origin: origin,
    })
}

#[test]
fn a_linked_worktree_without_its_own_overlay_shares_the_main_worktrees() {
    let worktrees = locator(linked());
    let store = MemoryStore::with(&[(MAIN, "{}")]);
    let selected =
        SelectConfig::new(store, worktrees.clone()).execute(request(None, Some("/work")));
    assert_eq!(selected, layered(MAIN, OverlayOrigin::MainWorktree));
    assert_eq!(*worktrees.asked.lock().unwrap(), [PathBuf::from("/work")]);
}

#[test]
fn a_linked_worktree_with_its_own_overlay_keeps_it_and_git_is_not_consulted() {
    let worktrees = locator(linked());
    let store = MemoryStore::with(&[(OWN, "{}"), (MAIN, "{}")]);
    let selected =
        SelectConfig::new(store, worktrees.clone()).execute(request(None, Some("/work")));
    assert_eq!(selected, layered(OWN, OverlayOrigin::WorkingDirectory));
    assert!(worktrees.asked.lock().unwrap().is_empty());
}

/// An own overlay the policy refuses (a link) or cannot read is still the
/// worktree's own: it is reported as such, never skipped for the main's.
#[test]
fn an_own_overlay_that_is_refused_or_unreadable_is_still_the_worktrees_own() {
    for (symlink, broken) in [(true, false), (false, true)] {
        let mut store = MemoryStore::default();
        store
            .files
            .lock()
            .unwrap()
            .insert(MAIN.into(), b"{}".to_vec());
        if symlink {
            store.symlinks.insert(PathBuf::from("/work/.quecto"));
        }
        if broken {
            store
                .files
                .lock()
                .unwrap()
                .insert(OWN.into(), b"{}".to_vec());
            store.broken.insert(PathBuf::from(OWN));
        }
        let selected = SelectConfig::new(Arc::new(store), locator(linked()))
            .execute(request(None, Some("/work")));
        assert_eq!(selected, layered(OWN, OverlayOrigin::WorkingDirectory));
    }
}

/// With nothing to share, the candidate stays the worktree's own, so a
/// `config set` there writes the worktree's own file.
#[test]
fn a_linked_worktree_whose_main_has_no_overlay_keeps_its_own_location() {
    let selected = SelectConfig::new(MemoryStore::with(&[]), locator(linked()))
        .execute(request(None, Some("/work")));
    assert_eq!(selected, layered(OWN, OverlayOrigin::WorkingDirectory));
}

/// A refused main overlay (a link) is shared — and then refused when it is
/// read, by the same rule a run in the main worktree meets.
#[test]
fn a_main_overlay_the_policy_refuses_is_shared_so_its_refusal_is_reported() {
    let mut store = MemoryStore::default();
    store.symlinks.insert(PathBuf::from(MAIN));
    let selected =
        SelectConfig::new(Arc::new(store), locator(linked())).execute(request(None, Some("/work")));
    assert_eq!(selected, layered(MAIN, OverlayOrigin::MainWorktree));
}

#[test]
fn an_unresolved_main_worktree_fails_closed_to_the_own_location_with_its_reason() {
    let unresolved = WorktreeLink::Unresolved {
        reason: "broken gitdir".into(),
    };
    let store = MemoryStore::with(&[(MAIN, "{}")]);
    let selected =
        SelectConfig::new(store, locator(unresolved)).execute(request(None, Some("/work")));
    assert_eq!(
        selected,
        layered(
            OWN,
            OverlayOrigin::MainWorktreeUnresolved {
                reason: "broken gitdir".into()
            }
        )
    );
}

/// A main worktree whose overlay is the global file (a worktree of the
/// home directory's repository) shares nothing.
#[test]
fn the_global_file_is_never_shared_as_a_main_worktrees_overlay() {
    let worktrees = locator(WorktreeLink::Linked {
        main_worktree: PathBuf::from("/home/u"),
    });
    let store = MemoryStore::with(&[("/home/u/.quecto/config.json", "{}")]);
    let selected = SelectConfig::new(store, worktrees).execute(request(None, Some("/work")));
    assert_eq!(selected, layered(OWN, OverlayOrigin::WorkingDirectory));
}

#[test]
fn an_explicit_selection_or_no_working_directory_never_consults_git() {
    let worktrees = locator(linked());
    let select = SelectConfig::new(MemoryStore::with(&[(MAIN, "{}")]), worktrees.clone());
    select.execute(request(Some("/x/c.json"), Some("/work")));
    select.execute(request(None, None));
    assert!(worktrees.asked.lock().unwrap().is_empty());
}

fn request(explicit: Option<&str>, cwd: Option<&str>) -> ConfigSelectionRequest {
    ConfigSelectionRequest {
        explicit: explicit.map(PathBuf::from),
        working_directory: cwd.map(PathBuf::from),
        global: PathBuf::from("/home/u/.quecto/config.json"),
    }
}

#[test]
fn an_explicit_override_replaces_both_layers() {
    let selected = select().execute(request(Some("/x/c.json"), Some("/work")));
    assert_eq!(
        selected,
        ConfigSelection::Explicit(PathBuf::from("/x/c.json"))
    );
}

#[test]
fn the_working_directory_names_the_overlay_and_the_retired_local_file() {
    let selected = select().execute(request(None, Some("/work")));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: Some(PathBuf::from("/work/.quecto/config.json")),
            legacy_local: Some(PathBuf::from("/work/config.json")),
            overlay_origin: OverlayOrigin::WorkingDirectory,
        })
    );
}

#[test]
fn without_a_working_directory_only_the_global_file_is_selected() {
    let selected = select().execute(request(None, None));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: None,
            legacy_local: None,
            overlay_origin: OverlayOrigin::WorkingDirectory,
        })
    );
}

/// A run from the base directory's parent (typically `$HOME`) or from the
/// base directory itself must not see the global file as its own overlay
/// or as a retired local file.
#[test]
fn the_global_file_is_never_its_own_overlay_or_legacy_candidate() {
    let selected = select().execute(request(None, Some("/home/u")));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: None,
            legacy_local: Some(PathBuf::from("/home/u/config.json")),
            overlay_origin: OverlayOrigin::WorkingDirectory,
        })
    );
    let selected = select().execute(request(None, Some("/home/u/.quecto")));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: Some(PathBuf::from("/home/u/.quecto/.quecto/config.json")),
            legacy_local: None,
            overlay_origin: OverlayOrigin::WorkingDirectory,
        })
    );
    let selected = select().execute(request(None, Some("/home/u/src/../")));
    assert_eq!(selected.overlay_path(), None, "normalised before comparing");
}
