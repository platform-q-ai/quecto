use super::*;

// Skills Steps
// ===========================================================================

/// Helper: ensure workspace skill dirs exist.
fn ensure_skill_dirs(world: &mut QuectoWorld) {
    if world.skill_loader_workspace.is_none() {
        let ws = TempDir::new().expect("temp dir");
        world.skill_loader_workspace = Some(ws.path().to_path_buf());
        world._skill_temp_dirs.push(ws);
    }
}

fn build_skill_loader(world: &QuectoWorld) -> FileSkillLoader {
    FileSkillLoader::new(world.skill_loader_workspace.as_ref().expect("ws"))
}

// --- Given steps ---

#[given(expr = "a workspace skill directory {string} with SKILL.md:")]
fn given_workspace_skill_with_frontmatter(
    world: &mut QuectoWorld,
    name: String,
    step: &gherkin::Step,
) {
    ensure_skill_dirs(world);
    let content = step.docstring.as_ref().expect("missing docstring").trim();
    let ws = world.skill_loader_workspace.as_ref().unwrap();
    let skill_dir = ws.join("skills").join(&name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), content).expect("write SKILL.md");
}

#[given(expr = "a workspace skill directory {string} without SKILL.md")]
fn given_workspace_skill_no_md(world: &mut QuectoWorld, name: String) {
    ensure_skill_dirs(world);
    let ws = world.skill_loader_workspace.as_ref().unwrap();
    let skill_dir = ws.join("skills").join(&name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
}

#[given(expr = "a workspace with skill {string} installed")]
fn given_workspace_skill_installed(world: &mut QuectoWorld, name: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let skill_dir = base.join("workspace").join("skills").join(&name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    let content = format!(
        "---\nname: {}\ndescription: {} skill\n---\n{} skill content",
        name, name, name
    );
    std::fs::write(skill_dir.join("SKILL.md"), content).expect("write SKILL.md");
}

#[given("an empty skill loader")]
fn given_empty_skill_loader(world: &mut QuectoWorld) {
    ensure_skill_dirs(world);
}

// --- When steps ---

#[when("the skill loader lists all skills")]
fn when_skills_list(world: &mut QuectoWorld) {
    let loader = build_skill_loader(world);
    world.skill_list = Some(loader.list().unwrap());
}

#[when(expr = "the skill {string} is loaded by name")]
fn when_skill_loaded_by_name(world: &mut QuectoWorld, name: String) {
    let loader = build_skill_loader(world);
    world.loaded_skill = Some(loader.load(&name).unwrap());
}

// --- Then steps ---

#[then(expr = "the skill list should contain {int} skill")]
fn then_skill_list_count_singular(world: &mut QuectoWorld, expected: usize) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    assert_eq!(
        skills.len(),
        expected,
        "expected {} skills, got {}",
        expected,
        skills.len()
    );
}

#[then(expr = "the skill list should contain {int} skills")]
fn then_skill_list_count(world: &mut QuectoWorld, expected: usize) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    assert_eq!(
        skills.len(),
        expected,
        "expected {} skills, got {}",
        expected,
        skills.len()
    );
}

#[then(expr = "the skill list should include {string}")]
fn then_skill_list_includes(world: &mut QuectoWorld, name: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    assert!(
        skills.iter().any(|s| s.name == name),
        "skill list should include '{}', has: {:?}",
        name,
        skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[then(expr = "the skill {string} should have description {string}")]
fn then_skill_has_description(world: &mut QuectoWorld, name: String, expected: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("skill '{}' not found in list", name));
    assert_eq!(
        skill.description, expected,
        "expected skill '{}' description '{}', got '{}'",
        name, expected, skill.description
    );
}

#[then("the loaded skill should exist")]
fn then_loaded_skill_exists(world: &mut QuectoWorld) {
    let loaded = world.loaded_skill.as_ref().expect("no load was performed");
    assert!(loaded.is_some(), "expected skill to be found");
}

#[then("the loaded skill should not exist")]
fn then_loaded_skill_not_exists(world: &mut QuectoWorld) {
    let loaded = world.loaded_skill.as_ref().expect("no load was performed");
    assert!(loaded.is_none(), "expected skill to not be found");
}

#[then(expr = "the loaded skill content should contain {string}")]
fn then_loaded_skill_content(world: &mut QuectoWorld, expected: String) {
    let loaded = world
        .loaded_skill
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("skill should be found");
    assert!(
        loaded.content.contains(&expected),
        "expected skill content to contain '{}', got: {}",
        expected,
        loaded.content
    );
}

#[then(expr = "the loaded skill content should not contain {string}")]
fn then_loaded_skill_content_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let loaded = world
        .loaded_skill
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("skill should be found");
    assert!(
        !loaded.content.contains(&unexpected),
        "expected skill content to NOT contain '{}', got: {}",
        unexpected,
        loaded.content
    );
}

// ===========================================================================
