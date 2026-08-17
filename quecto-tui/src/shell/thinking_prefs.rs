use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::tab_registry::tui_data_dir;

const THINKING_PREFS_FILE_NAME: &str = "thinking-preferences.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThinkingPreferences {
    pub visible: bool,
}

impl Default for ThinkingPreferences {
    fn default() -> Self {
        Self { visible: true }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingPreferencesFile {
    visible: bool,
}

pub(crate) fn default_thinking_preferences_path() -> PathBuf {
    tui_data_dir().join(THINKING_PREFS_FILE_NAME)
}

pub(crate) fn load_thinking_preferences_from(path: &Path) -> ThinkingPreferences {
    let Ok(bytes) = fs::read(path) else {
        return ThinkingPreferences::default();
    };
    serde_json::from_slice::<ThinkingPreferencesFile>(&bytes)
        .map(|file| ThinkingPreferences {
            visible: file.visible,
        })
        .unwrap_or_default()
}

pub(crate) fn store_thinking_preferences_to(path: &Path, prefs: ThinkingPreferences) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = fs::create_dir_all(parent);
    if let Ok(bytes) = serde_json::to_vec_pretty(&ThinkingPreferencesFile {
        visible: prefs.visible,
    }) {
        let _ = fs::write(path, bytes);
    }
}

pub(crate) fn load_thinking_preferences() -> ThinkingPreferences {
    load_thinking_preferences_from(&default_thinking_preferences_path())
}

pub(crate) fn store_thinking_preferences(prefs: ThinkingPreferences) {
    store_thinking_preferences_to(&default_thinking_preferences_path(), prefs);
}
