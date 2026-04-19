//! Contract tests for the `SkillLoader` port.

use quecto::domain::skill::SkillLoader;
use quecto::infrastructure::persistence::skill_loader::FileSkillLoader;
use std::path::Path;
use std::sync::Arc;

fn under_test(workspace: &Path) -> Arc<dyn SkillLoader> {
    Arc::new(FileSkillLoader::new(workspace))
}

fn seed_skill(workspace: &Path, name: &str, desc: &str, body: &str) {
    let dir = workspace.join("skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let content = format!("---\nname: {name}\ndescription: {desc}\n---\n{body}");
    std::fs::write(dir.join("SKILL.md"), content).unwrap();
}

#[test]
fn empty_workspace_yields_empty_skill_list() {
    let ws = tempfile::tempdir().unwrap();
    let loader = under_test(ws.path());
    assert_eq!(loader.list().unwrap().len(), 0);
}

#[test]
fn list_and_load_agree_on_the_same_skill() {
    let ws = tempfile::tempdir().unwrap();
    seed_skill(ws.path(), "weather", "Weather forecasts", "Body.");
    let loader = under_test(ws.path());

    let listed = loader.list().unwrap();
    assert_eq!(listed.len(), 1);
    let from_list = &listed[0];

    let from_load = loader.load("weather").unwrap()
        .expect("load must return Some for a listed skill");

    assert_eq!(from_list.name, from_load.name);
    assert_eq!(from_list.description, from_load.description);
    assert_eq!(from_list.content, from_load.content);
}

#[test]
fn load_returns_none_for_unknown_skill() {
    let ws = tempfile::tempdir().unwrap();
    let loader = under_test(ws.path());
    assert!(loader.load("no-such-skill").unwrap().is_none());
}

#[test]
fn load_rejects_invalid_skill_names() {
    // The port contract includes input validation (see `is_valid_skill_name`
    // in domain/skill.rs): invalid names must not be dispatched to disk.
    let ws = tempfile::tempdir().unwrap();
    let loader = under_test(ws.path());
    // Path-traversal attempt must not resolve:
    assert!(loader.load("../etc").unwrap().is_none());
    // Uppercase is invalid:
    assert!(loader.load("Weather").unwrap().is_none());
    // Empty is invalid:
    assert!(loader.load("").unwrap().is_none());
}
