//! #2098 dependency-hygiene ratchets: the three architecture.feature scenarios.
//! These assertions intentionally exercise the classifiers with hostile fixtures,
//! rather than relying on the current manifest alone to prove they can fail.

use std::fs;
use std::path::Path;

const ARCHIVE_PACKAGES: &[&str] = &["flate2", "tar"];
const INSTALLERS: &[(&str, &str)] = &[
    ("rg", "github.com/BurntSushi/ripgrep#installation"),
    ("fd", "github.com/sharkdp/fd#installation"),
];

fn section<'a>(manifest: &'a str, name: &str) -> &'a str {
    let header = format!("[{name}]");
    let Some(start) = manifest.lines().position(|line| line.trim() == header) else {
        return "";
    };
    let remainder = manifest.lines().skip(start + 1).collect::<Vec<_>>();
    let end = remainder
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .unwrap_or(remainder.len());
    // Preserve the exact lines while classifying only section-local assignments.
    let start_offset = manifest
        .lines()
        .take(start + 1)
        .map(|line| line.len() + 1)
        .sum::<usize>();
    let byte_len = remainder[..end]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    &manifest[start_offset.min(manifest.len())..(start_offset + byte_len).min(manifest.len())]
}

fn has_dependency(section: &str, name: &str) -> bool {
    section.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        line.split_once('=')
            .is_some_and(|(key, _)| key.trim() == name)
    })
}

fn normal_build_is_clean(manifest: &str, installer_module: &str, installer_exists: bool) -> bool {
    !installer_exists
        && !installer_module
            .lines()
            .filter_map(|line| line.split("//").next())
            .any(|line| line.trim() == "pub mod ensure_tool;")
        && ARCHIVE_PACKAGES
            .iter()
            .all(|package| !has_dependency(section(manifest, "dependencies"), package))
}

fn macos_normalization_is_scoped(manifest: &str) -> bool {
    !has_dependency(section(manifest, "dependencies"), "unicode-normalization")
        && has_dependency(
            section(manifest, "target.'cfg(target_os = \"macos\")'.dependencies"),
            "unicode-normalization",
        )
}

fn has_install_guidance(source: &str, binary: &str, url: &str) -> bool {
    source.contains(&format!("{binary} not found on PATH")) && source.contains(url)
}

#[test]
fn retired_installer_and_archive_packages_are_absent_from_normal_build() {
    let manifest = fs::read_to_string("Cargo.toml").expect("harness manifest");
    let module = fs::read_to_string("src/infrastructure/tools/mod.rs").expect("tools module");
    assert!(normal_build_is_clean(
        &manifest,
        &module,
        Path::new("src/infrastructure/tools/ensure_tool.rs").exists()
    ));
    assert!(!normal_build_is_clean(
        "[dependencies]\ntar = \"0.4\"\n",
        "",
        false
    ));
    assert!(!normal_build_is_clean(
        "[dependencies]\nflate2 = \"1\"\n",
        "",
        false
    ));
    assert!(!normal_build_is_clean(
        "[dependencies]\n",
        "pub mod ensure_tool;",
        false
    ));
    assert!(!normal_build_is_clean("[dependencies]\n", "", true));
    assert!(normal_build_is_clean(
        "[dependencies]\n# tar = \"0.4\"\n[target.'cfg(unix)'.dependencies]\ntar = \"0.4\"\n",
        "// pub mod ensure_tool;",
        false
    ));
}

#[test]
fn search_tools_retain_direct_install_guidance() {
    for (path, binary, url) in [
        (
            "src/infrastructure/tools/grep.rs",
            INSTALLERS[0].0,
            INSTALLERS[0].1,
        ),
        (
            "src/infrastructure/tools/find_fd.rs",
            INSTALLERS[1].0,
            INSTALLERS[1].1,
        ),
    ] {
        let source = fs::read_to_string(path).unwrap_or_else(|err| panic!("{path}: {err}"));
        assert!(
            has_install_guidance(&source, binary, url),
            "{path} lost {binary} install guidance"
        );
        assert!(!has_install_guidance(
            "binary missing; install it",
            binary,
            url
        ));
        assert!(!has_install_guidance(
            &format!("{binary} not found on PATH"),
            binary,
            url
        ));
        assert!(!has_install_guidance(url, binary, url));
    }
}

#[test]
fn unicode_normalization_is_macos_only() {
    let manifest = fs::read_to_string("Cargo.toml").expect("harness manifest");
    assert!(macos_normalization_is_scoped(&manifest));
    assert!(!macos_normalization_is_scoped(
        "[dependencies]\nunicode-normalization = \"0.1\"\n[target.'cfg(target_os = \"macos\")'.dependencies]\nunicode-normalization = \"0.1\"\n"
    ));
    assert!(!macos_normalization_is_scoped("[dependencies]\n"));
    assert!(macos_normalization_is_scoped(
        "[dependencies]\n# unicode-normalization = \"0.1\"\n[target.'cfg(target_os = \"macos\")'.dependencies]\nunicode-normalization = \"0.1\"\n"
    ));
}
