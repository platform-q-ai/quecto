use super::*;

#[then(expr = "Cargo.toml should contain a release profile with {string}")]
fn then_cargo_toml_has_release_profile_with(_world: &mut QuectoWorld, expected: String) {
    let content = std::fs::read_to_string("../Cargo.toml").expect("read workspace Cargo.toml");

    // Find the [profile.release] section
    let section_start = content
        .find("[profile.release]")
        .expect("Cargo.toml should contain a [profile.release] section");

    // Extract everything from [profile.release] to the next section header or EOF
    let rest = &content[section_start..];
    let section_end = rest[1..] // skip the opening '[' of [profile.release]
        .find("\n[")
        .map(|pos| pos + 1)
        .unwrap_or(rest.len());
    let section = &rest[..section_end];

    assert!(
        section.contains(&expected),
        "[profile.release] section should contain '{}', but it was:\n{}",
        expected,
        section,
    );
}

#[then(expr = "Cargo.toml should not contain a direct dependency on {string}")]
fn then_cargo_toml_no_direct_dep(_world: &mut QuectoWorld, crate_name: String) {
    let content = std::fs::read_to_string("Cargo.toml").expect("read Cargo.toml");

    // Extract the [dependencies] section
    let dep_start = content
        .find("[dependencies]")
        .expect("Cargo.toml should contain a [dependencies] section");
    let rest = &content[dep_start..];
    let dep_end = rest["[dependencies]".len()..]
        .find("\n[")
        .map(|pos| pos + "[dependencies]".len())
        .unwrap_or(rest.len());
    let dep_section = &rest[..dep_end];

    // Check each non-comment line for the crate name as a dependency key
    for line in dep_section.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() || trimmed.starts_with('[') {
            continue;
        }
        // Dependency lines look like: `crate_name = ...` or `crate_name = { ... }`
        if let Some(key) = trimmed.split('=').next() {
            if key.trim() == crate_name {
                panic!(
                    "[dependencies] should not contain '{}', but found line: {}",
                    crate_name, trimmed,
                );
            }
        }
    }
}
