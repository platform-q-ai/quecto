// Shared path resolution utilities for tool implementations.
// Mirrors Pi's path-utils.js — handles ~ expansion, absolute paths,
// @ prefix stripping, Unicode space normalisation, and macOS filename fixups.
//
// # Security
// `resolve_to_cwd` and `resolve_read_path` return a `PathBuf` that may point
// outside the workspace (e.g. absolute paths, ~ expansion). Callers **must**
// pass the result through `Sandbox::validate_path()` before any I/O.
// The sandbox performs canonicalisation and workspace-boundary checks.
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
/// Note: `@` is a valid filename character. This stripping matches Pi's
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
/// The returned path may contain `..` traversal components — callers must
/// still validate through `Sandbox::validate_path()`.
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
/// # Security
/// The returned path **must** be validated by `Sandbox::validate_path()`
/// before any I/O — this function performs no sandbox checks.
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
/// # Security
/// The returned path **must** be validated by `Sandbox::validate_path()`
/// before any I/O. Existence probing happens only on the primary-resolved
/// path's parent directory, but `Sandbox` canonicalisation is still required.
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        TempDir::new().unwrap()
    }

    // --- normalize_at_prefix ---

    #[test]
    fn strips_at_prefix() {
        assert_eq!(normalize_at_prefix("@src/main.rs"), "src/main.rs");
    }

    #[test]
    fn no_at_prefix_unchanged() {
        assert_eq!(normalize_at_prefix("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn double_at_strips_one() {
        assert_eq!(normalize_at_prefix("@@foo"), "@foo");
    }

    // --- expand_tilde ---

    #[test]
    fn tilde_alone_expands_to_home() {
        if let Some(home) = home_dir() {
            let result = expand_tilde("~");
            assert_eq!(result, home);
        }
    }

    #[test]
    fn tilde_slash_expands() {
        if let Some(home) = home_dir() {
            let result = expand_tilde("~/foo/bar");
            assert_eq!(result, home.join("foo/bar"));
        }
    }

    #[test]
    fn no_tilde_unchanged() {
        let result = expand_tilde("foo/bar");
        assert_eq!(result, PathBuf::from("foo/bar"));
    }

    #[test]
    fn absolute_path_unchanged_by_tilde_expand() {
        let result = expand_tilde("/usr/bin/ls");
        assert_eq!(result, PathBuf::from("/usr/bin/ls"));
    }

    // --- normalize_unicode_spaces ---

    #[test]
    fn nbsp_normalised() {
        assert_eq!(
            normalize_unicode_spaces("my\u{00A0}file.txt"),
            "my file.txt"
        );
    }

    #[test]
    fn narrow_nbsp_normalised() {
        assert_eq!(
            normalize_unicode_spaces("my\u{202F}file.txt"),
            "my file.txt"
        );
    }

    #[test]
    fn regular_space_unchanged_borrowed() {
        let s = "my file.txt";
        let result = normalize_unicode_spaces(s);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, s);
    }

    #[test]
    fn ascii_fast_path_borrowed() {
        let s = "hello_world.rs";
        let result = normalize_unicode_spaces(s);
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn special_space_produces_owned() {
        let s = "my\u{3000}file.txt";
        let result = normalize_unicode_spaces(s);
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "my file.txt");
    }

    // --- resolve_to_cwd ---

    #[test]
    fn relative_path_resolved_against_cwd() {
        let td = tmp();
        let result = resolve_to_cwd("sub/file.txt", td.path());
        assert_eq!(result, td.path().join("sub/file.txt"));
    }

    #[test]
    fn absolute_path_returned_as_is() {
        let td = tmp();
        let result = resolve_to_cwd("/etc/hosts", td.path());
        assert_eq!(result, PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn tilde_resolved() {
        if let Some(home) = home_dir() {
            let td = tmp();
            let result = resolve_to_cwd("~/foo.txt", td.path());
            assert_eq!(result, home.join("foo.txt"));
        }
    }

    #[test]
    fn at_prefix_stripped_then_resolved() {
        let td = tmp();
        let result = resolve_to_cwd("@src/main.rs", td.path());
        assert_eq!(result, td.path().join("src/main.rs"));
    }

    #[test]
    fn nbsp_normalised_in_path() {
        let td = tmp();
        let result = resolve_to_cwd("my\u{00A0}file.txt", td.path());
        assert_eq!(result, td.path().join("my file.txt"));
    }

    #[test]
    fn dot_resolves_under_cwd() {
        let td = tmp();
        let result = resolve_to_cwd(".", td.path());
        assert_eq!(result, td.path().join("."));
    }

    // --- resolve_read_path ---

    #[test]
    fn existing_file_returned_directly() {
        let td = tmp();
        std::fs::write(td.path().join("readme.md"), "hello").unwrap();
        let result = resolve_read_path("readme.md", td.path());
        assert_eq!(result, td.path().join("readme.md"));
        assert!(result.exists());
    }

    #[test]
    fn non_existent_file_returns_primary() {
        let td = tmp();
        let result = resolve_read_path("missing.txt", td.path());
        assert_eq!(result, td.path().join("missing.txt"));
    }

    #[test]
    fn curly_right_quote_variant_found() {
        let td = tmp();
        let stored = td.path().join("Capture d'écran.png");
        std::fs::write(&stored, b"").unwrap();
        // Query with curly right quote (U+2019)
        let result = resolve_read_path("Capture d\u{2019}\u{E9}cran.png", td.path());
        assert_eq!(result, stored);
    }

    #[test]
    fn home_dir_cached() {
        // Call twice — should return same pointer (OnceLock)
        let a = home_dir();
        let b = home_dir();
        assert_eq!(a, b);
    }
}
