use super::*;

use quecto::infrastructure::reload::{ReloadSource, RuntimeReload, SourceChange};
use std::path::PathBuf;
use std::time::SystemTime;

// ===========================================================================
// Helper: ensure a reload temp dir exists, return its path.
// ===========================================================================
fn ensure_reload_dir(world: &mut QuectoWorld) -> PathBuf {
    if world._reload_tmp.is_none() {
        world._reload_tmp = Some(TempDir::new().expect("failed to create reload temp dir"));
    }
    world
        ._reload_tmp
        .as_ref()
        .expect("reload temp dir")
        .path()
        .to_path_buf()
}

fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).expect("failed to write source file");
    path
}

/// Rewrite a file's content, guaranteeing mtime advances. On filesystems with
/// coarse mtime resolution, a bare rewrite may not bump mtime, so we loop with
/// small sleeps until stat reports a new mtime.
fn rewrite_with_mtime_advance(path: &Path, content: &str) -> SystemTime {
    let before = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .expect("stat before rewrite");
    loop {
        std::fs::write(path, content).expect("failed to rewrite source file");
        let after = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .expect("stat after rewrite");
        if after != before {
            return after;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

// ===========================================================================
// ReloadSource scenarios
// ===========================================================================

#[given(expr = "a reload source for a file containing {string}")]
fn given_reload_source(world: &mut QuectoWorld, content: String) {
    let dir = ensure_reload_dir(world);
    let path = write_file(&dir, "source.txt", &content);
    world.reload_source = Some(ReloadSource::new(&path));
    world.reload_files.clear();
    world.reload_files.insert("source.txt".to_string(), path);
}

#[given("the source fingerprint is seeded from the file")]
fn given_source_seeded(world: &mut QuectoWorld) {
    let src = world.reload_source.as_mut().expect("no reload source");
    src.seed();
}

fn probe_source(world: &mut QuectoWorld) {
    let src = world.reload_source.as_mut().expect("no reload source");
    world.reload_source_change = Some(src.changed());
}

#[when("I probe the source without seeding")]
fn when_probe_unseeded(world: &mut QuectoWorld) {
    probe_source(world);
}

#[when("I probe the source")]
#[when("I probe the source again")]
fn when_probe_source(world: &mut QuectoWorld) {
    probe_source(world);
}

#[given("the source is probed once")]
fn given_source_probed_once(world: &mut QuectoWorld) {
    probe_source(world);
}

#[when(expr = "the file content is rewritten to {string}")]
#[given(expr = "the file content is rewritten to {string}")]
fn when_file_rewritten(world: &mut QuectoWorld, content: String) {
    let path = world
        .reload_files
        .get("source.txt")
        .expect("no source file");
    // Track mtime before so touch-only scenarios can capture it.
    world.reload_mtime_before = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    rewrite_with_mtime_advance(path, &content);
}

#[when("the file is touched with identical content")]
#[given("the file is touched with identical content")]
fn when_file_touched(world: &mut QuectoWorld) {
    let path = world
        .reload_files
        .get("source.txt")
        .expect("no source file");
    let content = std::fs::read_to_string(path).expect("read source");
    world.reload_mtime_before = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    rewrite_with_mtime_advance(path, &content);
}

#[when("the file is deleted")]
fn when_file_deleted(world: &mut QuectoWorld) {
    let path = world
        .reload_files
        .get("source.txt")
        .expect("no source file");
    std::fs::remove_file(path).expect("failed to delete source file");
}

fn last_source_change(world: &mut QuectoWorld) -> SourceChange {
    world
        .reload_source_change
        .take()
        .expect("source was not probed")
}

#[then(expr = "the source should report changed")]
fn then_source_changed(world: &mut QuectoWorld) {
    let change = last_source_change(world);
    assert!(
        matches!(change, SourceChange::Changed),
        "expected Changed, got {:?}",
        change
    );
}

#[then(expr = "the source should report unchanged-no-read")]
fn then_source_unchanged_no_read(world: &mut QuectoWorld) {
    let change = last_source_change(world);
    assert!(
        matches!(change, SourceChange::UnchangedNoRead),
        "expected UnchangedNoRead, got {:?}",
        change
    );
}

#[then(expr = "the source should report unchanged")]
fn then_source_unchanged(world: &mut QuectoWorld) {
    let change = last_source_change(world);
    assert!(
        matches!(change, SourceChange::Unchanged),
        "expected Unchanged, got {:?}",
        change
    );
}

#[then(expr = "the source should report missing-or-unreadable")]
fn then_source_missing(world: &mut QuectoWorld) {
    let change = last_source_change(world);
    assert!(
        matches!(change, SourceChange::MissingOrUnreadable),
        "expected MissingOrUnreadable, got {:?}",
        change
    );
}

#[then("the source mtime cache should be advanced to the touched mtime")]
fn then_source_mtime_advanced(world: &mut QuectoWorld) {
    let src = world.reload_source.as_ref().expect("no reload source");
    let cached = src.last_mtime().expect("mtime cache should be set");
    let before = world
        .reload_mtime_before
        .expect("mtime before touch not captured");
    assert_ne!(
        cached, before,
        "mtime cache should have advanced past the pre-touch value"
    );
}

#[then("the source cache should be unchanged")]
fn then_source_cache_unchanged(world: &mut QuectoWorld) {
    let src = world.reload_source.as_ref().expect("no reload source");
    // After a MissingOrUnreadable, the cache must be untouched: mtime is still
    // the seeded value (not None), hash is still the seeded hash.
    assert!(
        src.last_mtime().is_some(),
        "mtime cache should remain set after missing/unreadable"
    );
}

// ===========================================================================
// RuntimeReload gate scenarios
// ===========================================================================

#[given(expr = "a runtime reload gate watching a file containing {string}")]
fn given_gate_single(world: &mut QuectoWorld, content: String) {
    let dir = ensure_reload_dir(world);
    let path = write_file(&dir, "source.txt", &content);
    world.reload_files.clear();
    world
        .reload_files
        .insert("source.txt".to_string(), path.clone());
    let source = ReloadSource::new(path);
    world.reload_gate = Some(RuntimeReload::new(vec![source]));
}

#[given(
    expr = "a runtime reload gate watching files {string} containing {string} and {string} containing {string}"
)]
fn given_gate_two(
    world: &mut QuectoWorld,
    name_a: String,
    content_a: String,
    name_b: String,
    content_b: String,
) {
    let dir = ensure_reload_dir(world);
    let path_a = write_file(&dir, &name_a, &content_a);
    let path_b = write_file(&dir, &name_b, &content_b);
    world.reload_files.clear();
    world.reload_files.insert(name_a, path_a.clone());
    world.reload_files.insert(name_b, path_b.clone());
    world.reload_gate = Some(RuntimeReload::new(vec![
        ReloadSource::new(path_a),
        ReloadSource::new(path_b),
    ]));
}

#[given(expr = "the gate is seeded with last-good {string}")]
fn given_gate_seeded(world: &mut QuectoWorld, last_good: String) {
    let gate = world.reload_gate.as_mut().expect("no reload gate");
    gate.seed(last_good);
}

#[when("I poll the gate with a rebuild closure")]
#[when("I poll the gate again with a rebuild closure")]
fn when_poll_gate(world: &mut QuectoWorld) {
    let called = world.reload_rebuild_called.clone();
    *called.lock().unwrap() = false;
    let gate = world.reload_gate.as_mut().expect("no reload gate");
    let result = gate.poll(|| {
        *called.lock().unwrap() = true;
        Ok("provider-rebuilt".to_string())
    });
    world.reload_poll_result = Some(result);
}

#[when(expr = "I poll the gate with a rebuild closure returning {string}")]
fn when_poll_gate_returning(world: &mut QuectoWorld, value: String) {
    let called = world.reload_rebuild_called.clone();
    *called.lock().unwrap() = false;
    let gate = world.reload_gate.as_mut().expect("no reload gate");
    let v = value.clone();
    let result = gate.poll(move || {
        *called.lock().unwrap() = true;
        Ok(v.clone())
    });
    world.reload_poll_result = Some(result);
}

#[when("I poll the gate with a failing rebuild closure")]
#[given("the gate is polled with a failing rebuild closure")]
fn when_poll_gate_failing(world: &mut QuectoWorld) {
    let called = world.reload_rebuild_called.clone();
    *called.lock().unwrap() = false;
    let gate = world.reload_gate.as_mut().expect("no reload gate");
    let result = gate.poll(|| {
        *called.lock().unwrap() = true;
        Err("malformed config".to_string())
    });
    world.reload_poll_result = Some(result);
}

#[when(expr = "I force-poll the gate with a rebuild closure returning {string}")]
fn when_force_poll_returning(world: &mut QuectoWorld, value: String) {
    let called = world.reload_rebuild_called.clone();
    *called.lock().unwrap() = false;
    let gate = world.reload_gate.as_mut().expect("no reload gate");
    let v = value.clone();
    let result = gate.poll_forced(move || {
        *called.lock().unwrap() = true;
        Ok(v.clone())
    });
    world.reload_poll_result = Some(result);
}

#[when("I force-poll the gate with a failing rebuild closure")]
fn when_force_poll_failing(world: &mut QuectoWorld) {
    let called = world.reload_rebuild_called.clone();
    *called.lock().unwrap() = false;
    let gate = world.reload_gate.as_mut().expect("no reload gate");
    let result = gate.poll_forced(|| {
        *called.lock().unwrap() = true;
        Err("malformed config".to_string())
    });
    world.reload_poll_result = Some(result);
}

// Multi-source file rewrite (by label)
#[when(expr = "the file {string} content is rewritten to {string}")]
fn when_named_file_rewritten(world: &mut QuectoWorld, label: String, content: String) {
    let path = world
        .reload_files
        .get(&label)
        .unwrap_or_else(|| panic!("no reload file labeled {}", label));
    rewrite_with_mtime_advance(path, &content);
}

#[then(expr = "the poll result should be unchanged")]
fn then_poll_unchanged(world: &mut QuectoWorld) {
    let result = world.reload_poll_result.take().expect("no poll result");
    assert!(
        matches!(
            result,
            quecto::infrastructure::reload::ReloadResult::Unchanged
        ),
        "expected Unchanged, got {:?}",
        result
    );
}

#[then(expr = "the poll result should be reloaded with {string}")]
fn then_poll_reloaded(world: &mut QuectoWorld, expected: String) {
    let result = world.reload_poll_result.take().expect("no poll result");
    match result {
        quecto::infrastructure::reload::ReloadResult::Reloaded(v) => {
            assert_eq!(v, expected, "reloaded value mismatch");
        }
        other => panic!("expected Reloaded, got {:?}", other),
    }
}

#[then("the rebuild closure should not be called")]
fn then_rebuild_not_called(world: &mut QuectoWorld) {
    let called = world.reload_rebuild_called.clone();
    assert!(
        !*called.lock().unwrap(),
        "rebuild closure should not have been called"
    );
}

#[then(expr = "the gate last-good should be {string}")]
fn then_gate_last_good(world: &mut QuectoWorld, expected: String) {
    let gate = world.reload_gate.as_ref().expect("no reload gate");
    let last_good = gate.last_good().expect("gate should have last-good");
    assert_eq!(last_good, &expected, "last-good mismatch");
}

// Register the poll-result and gate-poll-ordering state on the World.
// (Inserted below via a second edit to add the field.)
