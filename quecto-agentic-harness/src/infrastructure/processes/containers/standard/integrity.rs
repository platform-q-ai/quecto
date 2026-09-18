//! The launch policy's [`ContainerScriptIntegrity`] port (#2024 S4e)
//! over the embedded standard bundle: a script whose path is
//! `<project>/.quecto/containers/standard/<asset>` is judged against the
//! embedded bytes with the same observation `init` and `status` use
//! (identical, differing, missing; a symbolic link on the way refused),
//! so what `status` prints as `differs` is what a launch refuses. Any
//! other path is not the bundle's and gets no verdict.

use std::path::Path;

use crate::application::environments::dto::{AssetState, STANDARD_CONTAINER_DIR};
use crate::application::environments::ports::ContainerAssetStore;
use crate::application::subagents::dto::StandardScriptVerdict;
use crate::application::subagents::ports::ContainerScriptIntegrity;

use super::assets::EmbeddedStandardAssets;

#[derive(Debug, Default, Clone, Copy)]
pub struct EmbeddedScriptIntegrity;

impl ContainerScriptIntegrity for EmbeddedScriptIntegrity {
    fn verify(&self, script: &Path) -> StandardScriptVerdict {
        let (root, relative) = match split_at_bundle(script) {
            Ok(split) => split,
            Err(verdict) => return verdict,
        };
        let store = EmbeddedStandardAssets;
        let catalogue = store.catalogue();
        let Some(asset) = catalogue
            .assets
            .iter()
            .find(|asset| Path::new(&asset.path) == relative)
        else {
            return StandardScriptVerdict::NotStandard;
        };
        let dir = root.join(STANDARD_CONTAINER_DIR);
        match store.observe(root, &dir, asset) {
            Ok(AssetState::Identical) => StandardScriptVerdict::Intact,
            Ok(AssetState::Differs) => StandardScriptVerdict::Differs,
            Ok(AssetState::Missing) => StandardScriptVerdict::Missing,
            Ok(AssetState::Refused) => {
                StandardScriptVerdict::Refused("the destination is not a regular file".into())
            }
            Err(reason) => StandardScriptVerdict::Refused(reason),
        }
    }
}

/// A retained argv's program judged before the host runs it (review
/// round 2 of #2024 S4e): the environment's `exec`, `inspect`, `kill` and
/// `cleanup` argv were retained at create time, so a standard-bundle
/// script altered since then would otherwise run unjudged. The error is
/// the same text the create prints, naming the file and the refresh.
pub fn refuse_altered_script(argv: &[String]) -> Result<(), String> {
    let Some(program) = argv.first() else {
        return Ok(());
    };
    let script = Path::new(program);
    match EmbeddedScriptIntegrity.verify(script).refusal(script) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// `<root>/.quecto/containers/standard/<relative>` split at the bundle
/// directory: the project root and the asset's path within the bundle.
/// Only an absolute path with the bundle components exactly qualifies;
/// one that reaches the bundle through `..` anywhere is refused
/// rather than passed as not the bundle's, since it may name an asset
/// under another spelling.
fn split_at_bundle(script: &Path) -> Result<(&Path, &Path), StandardScriptVerdict> {
    if !script.is_absolute() {
        return Err(StandardScriptVerdict::NotStandard);
    }
    let mut ancestor = script.parent();
    while let Some(dir) = ancestor {
        if dir.ends_with(STANDARD_CONTAINER_DIR) {
            let normalised = script.components().all(|c| {
                matches!(
                    c,
                    std::path::Component::Normal(_)
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            });
            if !normalised {
                return Err(StandardScriptVerdict::Refused(
                    "path into the standard bundle is not normalised".into(),
                ));
            }
            let root = dir
                .ancestors()
                .nth(Path::new(STANDARD_CONTAINER_DIR).components().count())
                .ok_or(StandardScriptVerdict::NotStandard)?;
            let relative = script
                .strip_prefix(dir)
                .map_err(|_| StandardScriptVerdict::NotStandard)?;
            return Ok((root, relative));
        }
        ancestor = dir.parent();
    }
    Err(StandardScriptVerdict::NotStandard)
}

/// A materialised standard bundle and a way to alter one of its scripts,
/// for the tests of every path that must refuse an altered script.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    use crate::application::environments::dto::STANDARD_CONTAINER_DIR;
    use crate::application::environments::ports::ContainerAssetStore;
    use crate::infrastructure::processes::containers::standard::assets::EmbeddedStandardAssets;

    /// Every asset written under `<project>/.quecto/containers/standard`;
    /// the bundle directory.
    pub(crate) fn materialise_bundle(project: &Path) -> PathBuf {
        let dir = project.join(STANDARD_CONTAINER_DIR);
        for asset in EmbeddedStandardAssets.catalogue().assets {
            EmbeddedStandardAssets
                .materialise(project, &dir, &asset)
                .unwrap();
        }
        dir
    }

    /// The edit a pull or a hand could make: the script touches `marker`
    /// and stops before any runtime call, so "never ran" is a fact about
    /// the host.
    pub(crate) fn alter_script(script: &Path, marker: &Path) {
        let original = std::fs::read_to_string(script).unwrap();
        let (shebang, rest) = original.split_once('\n').unwrap();
        std::fs::write(
            script,
            format!("{shebang}\ntouch '{}'\nexit 99\n{rest}", marker.display()),
        )
        .unwrap();
    }
}

#[cfg(test)]
#[path = "integrity_tests.rs"]
mod tests;
