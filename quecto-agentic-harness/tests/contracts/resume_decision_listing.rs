//! Contract of the picker path of the resume decisions (#2011) over the REAL
//! adapters: the version a `list_sessions` row carries IS the token the
//! resume transaction accepts — for every home state, from a cold index and
//! from an index-seeded listing — and a well-formed but lying derived index
//! can influence neither exact-key resolution nor admission.
use super::resume_decision::{World, identity};
use quecto::application::sessions::dto::{
    ListedSession, ResumeIntent, ResumeOutcome, ResumeRequest, ResumeSavedSessionError,
    SessionListScope,
};
use quecto::application::sessions::ports::session_home::SessionHomeCatalogue;
use quecto::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use quecto::domain::session_home::SessionHomeScope;
use quecto::infrastructure::persistence::session_home_catalogue::FileSessionHomeCatalogue;
use std::os::unix::ffi::OsStrExt;

async fn rows(world: &World) -> Vec<ListedSession> {
    let listed = world.handles.list_sessions.list(SessionListScope::Global);
    listed.await.expect("global listing").sessions
}

fn row<'a>(rows: &'a [ListedSession], key: &str) -> &'a ListedSession {
    rows.iter()
        .find(|row| row.summary.key == key)
        .unwrap_or_else(|| panic!("{key} is listed"))
}

/// Scoped here, scoped elsewhere, a non-UTF-8 folder, a legacy record and an
/// uninterpretable sidecar.
async fn world_of_every_home_state() -> (World, Vec<&'static str>) {
    let world = World::new().await;
    world.saved_in("cli:mine", &world.here).await;
    let elsewhere = world.base.join("../elsewhere");
    world.saved_in("cli:theirs", &elsewhere).await;
    let odd = world
        .base
        .join("..")
        .join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    world.saved_in("cli:odd", &odd).await;
    world.save_transcript("cli:legacy").await;
    world
        .saved_in("cli:broken", &world.base.join("../broken"))
        .await;
    std::fs::write(world.layout.home_file(&identity("cli:broken")), b"{broken").unwrap();
    let keys = vec![
        "cli:mine",
        "cli:theirs",
        "cli:odd",
        "cli:legacy",
        "cli:broken",
    ];
    (world, keys)
}

#[tokio::test]
async fn a_listed_rows_version_is_the_authoritys_version_for_every_home_state() {
    let (world, keys) = world_of_every_home_state().await;
    let authority = FileSessionHomeCatalogue::with_store(world.store.clone());
    // Cold: no derived index exists yet. Seeded: the second listing reuses it.
    for pass in ["cold index", "index-seeded"] {
        let rows = rows(&world).await;
        let mut states = Vec::new();
        for key in &keys {
            let listed = row(&rows, key);
            let scope = authority.read(&identity(key)).unwrap();
            assert_eq!(
                listed.home_version(),
                HomeVersion::of(&identity(key), &scope),
                "{pass}: {key}"
            );
            states.push(match scope {
                SessionHomeScope::Scoped(_) => "scoped",
                SessionHomeScope::LegacyUnscoped => "legacy",
                SessionHomeScope::Unavailable(_) => "unavailable",
            });
        }
        assert_eq!(
            states,
            ["scoped", "scoped", "scoped", "legacy", "unavailable"],
            "{pass}: every state is really exercised"
        );
        let distinct: std::collections::BTreeSet<_> =
            rows.iter().map(ListedSession::home_version).collect();
        assert_eq!(distinct.len(), rows.len(), "{pass}: no two rows share one");
    }
    assert!(world.layout.home_catalogue_file().exists(), "it was seeded");
}

#[tokio::test]
async fn the_version_of_a_listed_eligible_row_restores_it() {
    let (world, _) = world_of_every_home_state().await;
    let rows = rows(&world).await;
    let listed = row(&rows, "cli:mine");
    assert!(listed.resume_eligible);
    let request = ResumeRequest {
        target: listed.summary.key.clone(),
        intent: ResumeIntent::Restore,
        expected_home_version: Some(listed.home_version()),
    };
    let (result, _) = world.request(&request).await;
    assert!(
        matches!(result, Ok(ResumeOutcome::Resumed(_))),
        "{result:?}"
    );
}

/// The index is valid JSON of the current version with every stamp intact; it
/// only claims that `cli:theirs` was saved HERE.
#[tokio::test]
async fn a_well_formed_lying_index_cannot_influence_exact_key_resolution() {
    let world = World::new().await;
    world
        .saved_in("cli:theirs", &world.base.join("../elsewhere"))
        .await;
    let honest = rows(&world).await;
    let honest_version = row(&honest, "cli:theirs").home_version();
    let index = world.layout.home_catalogue_file();
    let mut lying: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
    let here = world.here.to_str().unwrap();
    let scope = &mut lying["records"]["cli:theirs"]["home"]["scope"];
    assert_eq!(scope["kind"], "scoped", "{scope}");
    scope["execution_dir"] = here.into();
    scope["group_kind"] = "folder".into();
    scope["group_path"] = here.into();
    std::fs::write(&index, serde_json::to_vec(&lying).unwrap()).unwrap();
    let rows = rows(&world).await;
    let lied = row(&rows, "cli:theirs");
    assert_eq!(
        lied.home,
        SessionHomeScope::Scoped(quecto::domain::session_home::SessionHome {
            execution_dir: world.here.clone(),
            group: quecto::domain::session_home::WorkspaceGroup::Folder {
                directory: world.here.clone(),
            },
            provenance: quecto::domain::session_home::AssociationProvenance::SavedHere,
        }),
        "the lie really reaches the listing: the index was trusted there"
    );
    assert_ne!(lied.home_version(), honest_version);
    // Exact key: the authority alone decides.
    assert_eq!(
        world.decision("cli:theirs").await.kind,
        ResumeDecisionKind::CrossFolder
    );
    // The picker selection of the lying row: stale, and nothing happened.
    let files = world.files();
    let request = ResumeRequest {
        target: "cli:theirs".into(),
        intent: ResumeIntent::Restore,
        expected_home_version: Some(lied.home_version()),
    };
    let (result, messages) = world.request(&request).await;
    assert!(
        matches!(result, Err(ResumeSavedSessionError::StaleHomeVersion)),
        "{result:?}"
    );
    world
        .assert_untouched("cli:theirs", &messages, &files)
        .await;
}
