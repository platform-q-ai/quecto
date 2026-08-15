// Shared path resolution utilities for tool implementations.
// Mirrors Quecto's path-utils.js — handles ~ expansion, absolute paths,
// @ prefix stripping, Unicode space normalisation, and macOS filename fixups.
//
// # Path policy
// `resolve_to_cwd` and `resolve_read_path` return a `PathBuf` that may point
// outside the workspace (e.g. absolute paths, ~ expansion). Callers should pass
// the result through `Sandbox::validate_path()` before any I/O so all filesystem
// tools share the same path hook. Agent entrypoints no longer enable workspace
// confinement; explicit lower-level restricted sandboxes still can.
//
// # HOME not set
// `expand_tilde` returns `PathBuf::from("~")` (a literal path component) when
// `dirs::home_dir()` returns `None` (containers, CI without HOME).
// Callers that need hard failure should check `path.starts_with("~")` after
// resolution and return a `DomainError::Config`.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Strip a leading `@` prefix that editors sometimes prepend to file references.
///
/// Note: `@` is a valid filename character. This stripping matches Quecto's
/// `path-utils.js` behaviour and is intentional for editor-reference paths.
pub fn normalize_at_prefix(path: &str) -> &str {
    path.strip_prefix('@').unwrap_or(path)
}

/// Return the cached home directory (resolved once per process).
///
/// Returns `None` in environments where `HOME` is unset (containers, CI,
/// systemd units). Callers should treat `None` as an unresolvable tilde.
pub fn home_dir() -> Option<&'static Path> {
    static HOME: OnceLock<Option<PathBuf>> = OnceLock::new();
    HOME.get_or_init(dirs::home_dir).as_deref()
}

/// Expand `~` and `~/` to the home directory.
///
/// Returns the path unchanged (as `PathBuf::from(path)`) if it does not start
/// with `~`, or if the home directory cannot be determined.
///
/// The returned path may contain `..` traversal components. Callers should still
/// route it through `Sandbox::validate_path()` for consistent tool path policy.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir()
            .map(|h| h.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

const SPECIAL_SPACES: &[char] = &[
    '\u{00A0}', // NO-BREAK SPACE
    '\u{2002}', // EN SPACE
    '\u{2003}', // EM SPACE
    '\u{2004}', // THREE-PER-EM SPACE
    '\u{2005}', // FOUR-PER-EM SPACE
    '\u{2006}', // SIX-PER-EM SPACE
    '\u{2007}', // FIGURE SPACE
    '\u{2008}', // PUNCTUATION SPACE
    '\u{2009}', // THIN SPACE
    '\u{200A}', // HAIR SPACE
    '\u{202F}', // NARROW NO-BREAK SPACE
    '\u{205F}', // MEDIUM MATHEMATICAL SPACE
    '\u{3000}', // IDEOGRAPHIC SPACE
];

/// Normalise Unicode spaces to regular ASCII space (U+0020).
///
/// Single-pass: only allocates when a replacement is actually needed.
/// ASCII-only fast path avoids char decoding entirely.
pub fn normalize_unicode_spaces(s: &str) -> Cow<'_, str> {
    // ASCII fast path — most LLM-emitted paths are pure ASCII
    if s.bytes().all(|b| b.is_ascii()) {
        return Cow::Borrowed(s);
    }
    let mut buf: Option<String> = None;
    for (i, c) in s.char_indices() {
        if SPECIAL_SPACES.contains(&c) {
            let b = buf.get_or_insert_with(|| String::from(&s[..i]));
            b.push(' ');
        } else if let Some(b) = buf.as_mut() {
            b.push(c);
        }
    }
    match buf {
        Some(b) => Cow::Owned(b),
        None => Cow::Borrowed(s),
    }
}

/// Resolve a path relative to `cwd`, with `~` expansion, absolute path
/// support, `@` prefix stripping, and Unicode space normalisation.
///
/// Used by: write, edit, bash (indirectly), grep, find, ls.
///
/// # Path policy
/// The returned path should be routed through `Sandbox::validate_path()` before
/// any I/O for consistency with other tools. This function performs no
/// confinement checks.
pub fn resolve_to_cwd(path: &str, cwd: &Path) -> PathBuf {
    // 1. Strip @ prefix
    let path = normalize_at_prefix(path);
    // 2. Normalise Unicode spaces (borrows when no change needed)
    let normalised = normalize_unicode_spaces(path);
    let path = normalised.as_ref();
    // 3. Expand ~ and ~/
    let expanded = expand_tilde(path);
    // 4. If absolute, return as-is; if relative, resolve against cwd
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

/// Like `resolve_to_cwd` but tries macOS filename variants when the file
/// is not found at the primary resolved path.
///
/// Used by: read only.
///
/// Variants tried (in order):
/// 1. Primary resolved path (if it exists, return immediately)
/// 2. AM/PM narrow no-break space variant (U+202F before AM/PM)
/// 3. NFD Unicode decomposed form (macOS stores filenames in NFD)
/// 4. Curly right single quote → straight apostrophe (French screenshot names)
/// 5. Curly left single quote → straight apostrophe
///
/// Returns the first variant that exists on disk, or the primary path
/// if none exist.
///
/// # Path policy
/// The returned path should be routed through `Sandbox::validate_path()` before
/// any I/O for consistency with other tools. Existence probing happens only on
/// the primary-resolved path's parent directory; agent entrypoints perform no
/// workspace confinement here.
pub fn resolve_read_path(path: &str, cwd: &Path) -> PathBuf {
    let primary = resolve_to_cwd(path, cwd);

    if primary.exists() {
        return primary;
    }

    // Try macOS variants on the filename component only.
    if let Some(filename) = primary.file_name().and_then(|n| n.to_str()) {
        let parent = primary.parent().unwrap_or(Path::new(""));

        // Variant: narrow no-break space before AM/PM (screenshot filenames)
        let ampm_variant = filename
            .replace(" AM", "\u{202F}AM")
            .replace(" PM", "\u{202F}PM");
        if ampm_variant != filename {
            let candidate = parent.join(&ampm_variant);
            if candidate.exists() {
                return candidate;
            }
        }

        // Variant: NFD decomposition (macOS normalises filenames to NFD)
        #[cfg(target_os = "macos")]
        {
            use unicode_normalization::UnicodeNormalization;
            let nfd: String = filename.nfd().collect();
            if nfd != filename {
                let candidate = parent.join(&nfd);
                if candidate.exists() {
                    return candidate;
                }
            }
        }

        // Variant: curly right single quote → straight apostrophe
        let curly_right = filename.replace('\u{2019}', "'");
        if curly_right != filename {
            let candidate = parent.join(&curly_right);
            if candidate.exists() {
                return candidate;
            }
        }

        // Variant: curly left single quote → straight apostrophe
        let curly_left = filename.replace('\u{2018}', "'");
        if curly_left != filename {
            let candidate = parent.join(&curly_left);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    primary
}

#[cfg(test)]
#[path = "path_utils_tests.rs"]
mod tests;
