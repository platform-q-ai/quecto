use super::*;

#[then(expr = "Cargo.toml should contain a release profile with {string}")]
fn then_cargo_toml_has_release_profile_with(_world: &mut QuectoWorld, expected: String) {
    let content = std::fs::read_to_string("Cargo.toml").expect("read Cargo.toml");

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
