// Shared path resolution utilities for tool implementations.
// Mirrors Pi's path-utils.js — handles ~ expansion, absolute paths,
// @ prefix stripping, Unicode space normalisation, and macOS filename fixups.

use std::path::{Path, PathBuf};

/// Strip a leading `@` prefix that editors sometimes prepend to file references.
pub fn normalize_at_prefix(path: &str) -> &str {
    path.strip_prefix('@').unwrap_or(path)
}

/// Expand `~` and `~/` to the home directory.
/// Returns the path unchanged if it does not start with `~`.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Normalise Unicode spaces to regular ASCII space (U+0020).
///
/// Handles: non-breaking space (U+00A0), en/em/thin/hair spaces
/// (U+2002–U+200A), narrow no-break space (U+202F), medium mathematical
/// space (U+205F), ideographic space (U+3000).
pub fn normalize_unicode_spaces(s: &str) -> String {
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
    if s.chars().any(|c| SPECIAL_SPACES.contains(&c)) {
        s.chars()
            .map(|c| if SPECIAL_SPACES.contains(&c) { ' ' } else { c })
            .collect()
    } else {
        s.to_string()
    }
}

/// Resolve a path relative to `cwd`, with `~` expansion, absolute path
/// support, `@` prefix stripping, and Unicode space normalisation.
///
/// Used by: write, edit, bash (indirectly), grep, find, ls.
pub fn resolve_to_cwd(path: &str, cwd: &Path) -> PathBuf {
    // 1. Strip @ prefix
    let path = normalize_at_prefix(path);
    // 2. Normalise Unicode spaces
    let path = normalize_unicode_spaces(path);
    let path = path.as_str();
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
/// 1. Primary resolved path
/// 2. AM/PM narrow no-break space variant (U+202F before AM/PM)
/// 3. NFD Unicode decomposed form (macOS stores filenames in NFD)
/// 4. Curly-quote variant (U+2019 right single quotation mark)
/// 5. Combined NFD + curly-quote
///
/// Returns the first variant that exists on disk, or the primary path
/// if none exist.
pub fn resolve_read_path(path: &str, cwd: &Path) -> PathBuf {
    let primary = resolve_to_cwd(path, cwd);

    if primary.exists() {
        return primary;
    }

    // Try macOS variants — only meaningful on macOS (or cross-platform
    // when accessing macOS-generated files over a network share).
    if let Some(filename) = primary.file_name().and_then(|n| n.to_str()) {
        let parent = primary.parent().unwrap_or(Path::new(""));

        // Variant 1: narrow no-break space before AM/PM (screenshot filenames)
        let ampm_variant = filename
            .replace(" AM", "\u{202F}AM")
            .replace(" PM", "\u{202F}PM");
        if ampm_variant != filename {
            let candidate = parent.join(&ampm_variant);
            if candidate.exists() {
                return candidate;
            }
        }

        // Variant 2: NFD decomposition (macOS normalises filenames to NFD)
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

        // Variant 3: curly right single quote → straight apostrophe
        // (macOS uses U+2019 in French screenshot names like "Capture d'écran")
        let curly_variant = filename.replace('\u{2019}', "'");
        if curly_variant != filename {
            let candidate = parent.join(&curly_variant);
            if candidate.exists() {
                return candidate;
            }
        }

        // Variant 4: curly left single quote → straight apostrophe
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
        let result = expand_tilde("~");
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home);
    }

    #[test]
    fn tilde_slash_expands() {
        let result = expand_tilde("~/foo/bar");
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join("foo/bar"));
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
    fn regular_space_unchanged() {
        assert_eq!(normalize_unicode_spaces("my file.txt"), "my file.txt");
    }

    #[test]
    fn no_special_spaces_fast_path() {
        let s = "hello_world.rs";
        let result = normalize_unicode_spaces(s);
        assert_eq!(result, s);
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
        let td = tmp();
        let home = dirs::home_dir().unwrap();
        let result = resolve_to_cwd("~/foo.txt", td.path());
        assert_eq!(result, home.join("foo.txt"));
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
    fn dot_resolves_to_cwd() {
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
    fn curly_quote_variant_found() {
        let td = tmp();
        // File stored with straight apostrophe, looked up with curly quote
        let stored = td.path().join("Capture d'écran.png");
        std::fs::write(&stored, b"").unwrap();
        // Query with curly right quote (U+2019)
        let result = resolve_read_path("Capture d\u{2019}\u{E9}cran.png", td.path());
        // Should find the straight-apostrophe file
        assert_eq!(result, stored);
    }
}
