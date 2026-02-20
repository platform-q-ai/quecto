use super::*;

// Skills Steps
// ===========================================================================

/// Helper: ensure skill loader temp dirs exist.
fn ensure_skill_dirs(world: &mut QuectoWorld) {
    if world.skill_loader_workspace.is_none() {
        let ws = TempDir::new().expect("temp dir");
        let global = TempDir::new().expect("temp dir");
        let builtin = TempDir::new().expect("temp dir");
        world.skill_loader_workspace = Some(ws.path().to_path_buf());
        world.skill_loader_global = Some(global.path().to_path_buf());
        world.skill_loader_builtin = Some(builtin.path().to_path_buf());
        world._skill_temp_dirs.push(ws);
        world._skill_temp_dirs.push(global);
        world._skill_temp_dirs.push(builtin);
    }
}

fn create_workspace_skill(base: &Path, name: &str, content: Option<&str>) {
    let skill_dir = base.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    if let Some(c) = content {
        std::fs::write(skill_dir.join("SKILL.md"), c).expect("write SKILL.md");
    }
}

fn create_global_skill(base: &Path, name: &str, content: &str) {
    let skill_dir = base.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), content).expect("write SKILL.md");
}

fn create_builtin_skill_dir(base: &Path, name: &str, content: &str) {
    let skill_dir = base.join(name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), content).expect("write SKILL.md");
}

fn build_skill_loader(world: &QuectoWorld) -> FileSkillLoader {
    FileSkillLoader::new(
        world.skill_loader_workspace.as_ref().expect("ws"),
        world.skill_loader_global.as_ref().expect("global"),
        world.skill_loader_builtin.as_ref().expect("builtin"),
    )
}

#[given(expr = "a workspace with skill {string} installed")]
fn given_workspace_skill_installed(world: &mut QuectoWorld, name: String) {
    ensure_temp_dir(world);
    let skill_dir = base_path(world)
        .join("workspace")
        .join("skills")
        .join(&name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), format!("{} skill", name)).expect("write SKILL.md");
}

#[given(expr = "a skill loader with workspace skill {string} containing {string}")]
fn given_workspace_skill(world: &mut QuectoWorld, name: String, content: String) {
    ensure_skill_dirs(world);
    create_workspace_skill(
        world.skill_loader_workspace.as_ref().unwrap(),
        &name,
        Some(&content),
    );
}

#[given(expr = "a skill loader with global skill {string} containing {string}")]
fn given_global_skill(world: &mut QuectoWorld, name: String, content: String) {
    ensure_skill_dirs(world);
    create_global_skill(world.skill_loader_global.as_ref().unwrap(), &name, &content);
}

#[given(expr = "a skill loader with builtin skill {string} containing {string}")]
fn given_builtin_skill(world: &mut QuectoWorld, name: String, content: String) {
    ensure_skill_dirs(world);
    create_builtin_skill_dir(
        world.skill_loader_builtin.as_ref().unwrap(),
        &name,
        &content,
    );
}

#[given("an empty skill loader")]
fn given_empty_skill_loader(world: &mut QuectoWorld) {
    ensure_skill_dirs(world);
}

#[given(expr = "a skill loader with workspace skill {string} without SKILL.md")]
fn given_workspace_skill_no_md(world: &mut QuectoWorld, name: String) {
    ensure_skill_dirs(world);
    create_workspace_skill(world.skill_loader_workspace.as_ref().unwrap(), &name, None);
}

#[when("the skills loader lists all skills")]
fn when_skills_list(world: &mut QuectoWorld) {
    let loader = build_skill_loader(world);
    world.skill_list = Some(loader.list().unwrap());
}

#[when(expr = "the skill {string} is loaded by name")]
fn when_skill_loaded_by_name(world: &mut QuectoWorld, name: String) {
    let loader = build_skill_loader(world);
    world.loaded_skill = Some(loader.load(&name).unwrap());
}

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

#[then(expr = "the skill {string} should have source {string}")]
fn then_skill_has_source(world: &mut QuectoWorld, name: String, expected_source: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("skill '{}' not found in list", name));
    let source_str = match skill.source {
        SkillSource::Workspace => "workspace",
        SkillSource::Global => "global",
        SkillSource::Builtin => "builtin",
    };
    assert_eq!(
        source_str, expected_source,
        "expected skill '{}' source '{}', got '{}'",
        name, expected_source, source_str
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

#[then(expr = "the loaded skill should have source {string}")]
fn then_loaded_skill_source(world: &mut QuectoWorld, expected_source: String) {
    let loaded = world
        .loaded_skill
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("skill should be found");
    let source_str = match loaded.source {
        SkillSource::Workspace => "workspace",
        SkillSource::Global => "global",
        SkillSource::Builtin => "builtin",
    };
    assert_eq!(
        source_str, expected_source,
        "expected source '{}', got '{}'",
        expected_source, source_str
    );
}

#[then(expr = "the skill {string} should have empty content")]
fn then_skill_empty_content(world: &mut QuectoWorld, name: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("skill '{}' not found", name));
    assert!(
        skill.content.is_empty(),
        "expected skill '{}' to have empty content, got: {}",
        name,
        skill.content
    );
}

// ===========================================================================
