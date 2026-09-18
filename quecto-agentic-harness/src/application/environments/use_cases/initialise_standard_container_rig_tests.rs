//! The in-memory rig the `InitialiseStandardContainer` tests share: a
//! bundle on a map "disk", a fixed origin and toplevel, a fixed roster and
//! a recording persistence.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::InitialiseStandardContainer;
use crate::application::environments::dto::{
    AssetOutcome, AssetState, ContainerAsset, ContainerAssetCatalogue, ContainerConfigDocument,
    ContainerConfigEntry, ContainerConfigLayer, PersistedContainerConfig, StandardContainerRequest,
};
use crate::application::environments::ports::{
    ContainerAssetStore, ContainerConfigPersistence, ContainerConfigRoster,
    ContainerConfigRosterReport, WorkspaceOrigin,
};

/// An in-memory bundle: the "disk" is a map of path → bytes.
pub(super) struct MemoryAssets {
    pub(super) disk: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    /// Destinations `observe` refuses (a symbolic link in their place).
    pub(super) refuse: Mutex<std::collections::BTreeSet<PathBuf>>,
}

pub(super) fn catalogue() -> ContainerAssetCatalogue {
    ContainerAssetCatalogue {
        version: 7,
        build_command: "build -t {image} {dir}\n".into(),
        assets: vec![
            ContainerAsset {
                path: "Containerfile".into(),
                contents: b"FROM x".to_vec(),
                executable: false,
            },
            ContainerAsset {
                path: "scripts/create.sh".into(),
                contents: b"#!/bin/sh".to_vec(),
                executable: true,
            },
        ],
    }
}

impl ContainerAssetStore for MemoryAssets {
    fn catalogue(&self) -> ContainerAssetCatalogue {
        catalogue()
    }

    fn observe(&self, _: &Path, dir: &Path, asset: &ContainerAsset) -> Result<AssetState, String> {
        let path = dir.join(&asset.path);
        if self.refuse.lock().unwrap().contains(&path) {
            return Err(format!("{} is a symbolic link", path.display()));
        }
        Ok(match self.disk.lock().unwrap().get(&path) {
            None => AssetState::Missing,
            Some(bytes) if *bytes == asset.contents => AssetState::Identical,
            Some(_) => AssetState::Differs,
        })
    }

    fn materialise(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        assert!(
            dir.starts_with(root),
            "{} not below {}",
            dir.display(),
            root.display()
        );
        let state = self.observe(root, dir, asset)?;
        Ok(match state {
            AssetState::Missing => {
                self.disk
                    .lock()
                    .unwrap()
                    .insert(dir.join(&asset.path), asset.contents.clone());
                AssetOutcome::Written
            }
            AssetState::Identical => AssetOutcome::KeptIdentical,
            AssetState::Differs => AssetOutcome::KeptDiffering,
            AssetState::Refused => unreachable!(),
        })
    }

    fn refresh(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        match self.observe(root, dir, asset)? {
            AssetState::Differs => {
                self.disk
                    .lock()
                    .unwrap()
                    .insert(dir.join(&asset.path), asset.contents.clone());
                Ok(AssetOutcome::Refreshed)
            }
            _ => self.materialise(root, dir, asset),
        }
    }
}

pub(super) struct FixedOrigin {
    pub(super) origin: Result<Option<String>, String>,
    /// The toplevel git reports for any checkout asked about; `None` is
    /// "not a checkout".
    pub(super) toplevel: Mutex<Option<PathBuf>>,
}

impl WorkspaceOrigin for FixedOrigin {
    fn origin(&self, _: &Path) -> Result<Option<String>, String> {
        self.origin.clone()
    }

    fn toplevel(&self, _: &Path) -> Result<Option<PathBuf>, String> {
        Ok(self.toplevel.lock().unwrap().clone())
    }
}

pub(super) struct FixedRoster(pub(super) Result<ContainerConfigRosterReport, String>);

impl ContainerConfigRoster for FixedRoster {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
        self.0.clone()
    }

    fn revision(&self) -> String {
        String::new()
    }
}

#[derive(Default)]
pub(super) struct RecordingPersistence {
    pub(super) written: Mutex<Vec<(String, ContainerConfigDocument)>>,
    pub(super) refuse: Option<String>,
    pub(super) refuse_existing: Mutex<Option<String>>,
}

impl ContainerConfigPersistence for RecordingPersistence {
    fn check(&self) -> Result<(), String> {
        self.refuse.clone().map_or(Ok(()), Err)
    }

    fn persist(
        &self,
        name: &str,
        entry: &ContainerConfigDocument,
    ) -> Result<PersistedContainerConfig, String> {
        if let Some(reason) = &self.refuse {
            return Err(reason.clone());
        }
        self.written
            .lock()
            .unwrap()
            .push((name.to_string(), entry.clone()));
        Ok(PersistedContainerConfig {
            path: PathBuf::from("/p/.quecto/config.json"),
            created: true,
        })
    }

    fn location(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/p/.quecto/config.json"))
    }

    fn existing(&self, name: &str) -> Result<Option<ContainerConfigDocument>, String> {
        if let Some(reason) = self.refuse_existing.lock().unwrap().as_ref() {
            return Err(reason.clone());
        }
        Ok(self
            .written
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(written, _)| written == name)
            .map(|(_, entry)| entry.clone()))
    }
}

pub(super) fn entry(name: &str, default: bool) -> ContainerConfigEntry {
    ContainerConfigEntry {
        name: name.into(),
        default,
        layer: ContainerConfigLayer::Global,
        repository: None,
        problem: None,
        joinable: true,
    }
}

pub(super) struct Rig {
    pub(super) assets: Arc<MemoryAssets>,
    pub(super) persistence: Arc<RecordingPersistence>,
    pub(super) origin: Arc<FixedOrigin>,
    pub(super) use_case: InitialiseStandardContainer,
}

pub(super) fn build_rig(
    origin: Result<Option<String>, String>,
    roster: ContainerConfigRosterReport,
    refuse: Option<String>,
) -> Rig {
    let assets = Arc::new(MemoryAssets {
        disk: Mutex::new(BTreeMap::new()),
        refuse: Mutex::new(Default::default()),
    });
    let persistence = Arc::new(RecordingPersistence {
        written: Mutex::new(vec![]),
        refuse,
        refuse_existing: Mutex::new(None),
    });
    let origin = Arc::new(FixedOrigin {
        origin,
        toplevel: Mutex::new(Some(PathBuf::from("/p"))),
    });
    let use_case = InitialiseStandardContainer::new(
        assets.clone(),
        origin.clone(),
        Arc::new(FixedRoster(Ok(roster))),
        persistence.clone(),
        PathBuf::from("/base"),
    );
    Rig {
        assets,
        persistence,
        origin,
        use_case,
    }
}

pub(super) fn request(project: &str) -> StandardContainerRequest {
    StandardContainerRequest {
        project: PathBuf::from(project),
        repository: None,
        image: None,
        dry_run: false,
        refresh: false,
    }
}

pub(super) fn argv(entry: &ContainerConfigDocument, key: &str) -> Vec<String> {
    entry
        .argvs()
        .into_iter()
        .find(|(k, _)| *k == key)
        .map(|(_, argv)| argv.to_vec())
        .unwrap()
}
