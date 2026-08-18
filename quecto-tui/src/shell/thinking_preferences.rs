use std::path::PathBuf;

const ENV_KEY: &str = "QUECTO_TUI_THINKING_VISIBLE";
const FILE_NAME: &str = "thinking-visible";

pub(crate) fn load_thinking_visible() -> bool {
    if let Ok(value) = std::env::var(ENV_KEY) {
        return parse_bool(&value).unwrap_or(true);
    }
    preference_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|value| parse_bool(value.trim()))
        .unwrap_or(true)
}

pub(crate) fn save_thinking_visible(show: bool) {
    let Some(path) = preference_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, if show { "true\n" } else { "false\n" });
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "visible" => Some(true),
        "0" | "false" | "no" | "off" | "hidden" => Some(false),
        _ => None,
    }
}

fn preference_path() -> Option<PathBuf> {
    if let Ok(base) = std::env::var("QUECTO_TUI_STATE_DIR") {
        return Some(PathBuf::from(base).join(FILE_NAME));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".quecto").join("tui").join(FILE_NAME))
}
