//! The runtime-reach checker rejects every bypass review found (#2286
//! round 4), each fed to it as source appended to the real tree.

use super::{PIPES, SUPERVISOR, TASKS, access, parse_sources, process_sources};

/// The checker's findings over the real tree with `file` rewritten by
/// `edit`.
fn violations_after(file: &str, edit: impl Fn(&str) -> String) -> Vec<String> {
    let mut sources = process_sources();
    let (_, source) = sources
        .iter_mut()
        .find(|(path, _)| path == file)
        .unwrap_or_else(|| panic!("{file} is a processes file"));
    let edited = edit(source);
    assert_ne!(&edited, source, "the edit changes {file}");
    *source = edited;
    access::violations(&parse_sources(&sources))
}

fn with_appended(file: &str, extra: &str) -> Vec<String> {
    violations_after(file, |source| format!("{source}\n{extra}"))
}

/// Assert some finding names every one of `words`.
fn assert_rejected(found: &[String], words: &[&str]) {
    assert!(
        found
            .iter()
            .any(|finding| words.iter().all(|word| finding.contains(word))),
        "a finding names {words:?}; found:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_tree_as_it_is_reaches_the_runtime_only_through_its_allowlisted_helpers() {
    let found = access::violations(&parse_sources(&process_sources()));
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// Review round 4, bypass (a): a free function hands a caller's trait
/// object to the runtime handle.
#[test]
fn a_free_function_spawning_a_callers_trait_object_on_the_handle_is_rejected() {
    let found = with_appended(
        TASKS,
        "pub(crate) trait Work: Send { fn run(self: Box<Self>); }\n\
         pub(crate) fn run_work(supervisor: &OwnedChildSupervisor, work: Box<dyn Work>) {\n\
             supervisor.handle.spawn(async move { work.run() });\n\
         }",
    );
    assert_rejected(&found, &["`run_work`", "`.handle`"]);
    assert_rejected(&found, &["`run_work`", "dyn Work"]);
}

/// Review round 4, bypass (b): a supervisor method spawns through the
/// runtime itself rather than its handle.
#[test]
fn a_supervisor_method_spawning_through_the_runtime_field_is_rejected() {
    let found = with_appended(
        TASKS,
        "impl OwnedChildSupervisor {\n\
             pub(crate) fn run_tail(&self, stderr: tokio::process::ChildStderr) {\n\
                 let runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());\n\
                 if let Some(runtime) = runtime.as_ref() {\n\
                     runtime.spawn(async move { drop(stderr) });\n\
                 }\n\
             }\n\
         }",
    );
    assert_rejected(
        &found,
        &["`impl OwnedChildSupervisor::run_tail`", "`.runtime`"],
    );
}

#[test]
fn an_impl_on_a_type_containing_the_supervisor_is_covered() {
    let found = with_appended(
        TASKS,
        "trait Go { fn go(&self); }\n\
         impl Go for Arc<OwnedChildSupervisor> {\n\
             fn go(&self) { self.handle.spawn(async {}); }\n\
         }",
    );
    assert_rejected(
        &found,
        &["impl Go for Arc < OwnedChildSupervisor >::go", "`.handle`"],
    );
}

#[test]
fn an_allowlisted_name_outside_its_pinned_file_or_scope_is_rejected() {
    let found = with_appended(
        TASKS,
        "impl OwnedChildSupervisor {\n\
             fn spawn_pipe_task(&self) { self.handle.spawn(async {}); }\n\
         }",
    );
    assert_rejected(&found, &[TASKS, "spawn_pipe_task", "`.handle`"]);
    let found = with_appended(
        SUPERVISOR,
        "impl OwnedChildSupervisor {\n\
             fn outer(&self) { fn spawn(s: &OwnedChildSupervisor) { s.handle.spawn(async {}); } }\n\
         }",
    );
    assert_rejected(&found, &["outer::spawn", "`.handle`"]);
}

#[test]
fn destructuring_the_supervisor_reaches_its_runtime() {
    let found = with_appended(
        TASKS,
        "fn take(supervisor: &OwnedChildSupervisor) {\n\
             let OwnedChildSupervisor { handle, .. } = supervisor;\n\
             let _ = handle;\n\
         }",
    );
    assert_rejected(&found, &["`take`", "`handle`"]);
}

/// Code already running on the runtime reaches it without a field.
#[test]
fn spawning_from_code_on_the_runtime_is_rejected() {
    for (call, word) in [
        ("tokio::spawn(async {});", "tokio :: spawn"),
        ("tokio::task::spawn_blocking(|| ());", "spawn_blocking"),
        (
            "let _ = tokio::runtime::Handle::current();",
            "Handle :: current",
        ),
        (
            "let _ = tokio::runtime::Handle::try_current();",
            "try_current",
        ),
        ("tokio::task::JoinSet::new().spawn(async {});", "`.spawn`"),
        ("tokio::task::block_in_place(|| ());", "block_in_place"),
    ] {
        let found = with_appended(PIPES, &format!("async fn relay() {{ {call} }}"));
        assert_rejected(&found, &["`relay`", word]);
    }
    let found = with_appended(PIPES, "use tokio::task::spawn as go;");
    assert_rejected(&found, &["spawn"]);
}

#[test]
fn shapes_that_carry_code_are_rejected() {
    for (item, word) in [
        ("pub(super) struct Hook { run: fn() }", "fn ()"),
        (
            "fn carry<W: Send>(work: W) { drop(work) }",
            "type parameter W",
        ),
        ("fn carry(work: impl Send) { drop(work) }", "impl Send"),
        ("pub(super) struct Boxed(Box<dyn Send>);", "dyn Send"),
    ] {
        let found = with_appended(PIPES, item);
        assert_rejected(&found, &[word]);
    }
}

#[test]
fn code_hidden_from_syn_is_rejected() {
    let found = with_appended(
        TASKS,
        "macro_rules! go { ($s:expr) => { $s.handle.spawn(async {}) } }",
    );
    assert_rejected(&found, &["macro_rules"]);
    let found = with_appended(TASKS, "include!(\"elsewhere.rs\");");
    assert_rejected(&found, &["include"]);
    let found = with_appended(
        TASKS,
        "fn log(supervisor: &OwnedChildSupervisor) { tracing::info!(\"{:?}\", supervisor.handle.spawn(async {})); }",
    );
    assert_rejected(&found, &["`log`", "`.handle`"]);
}

#[test]
fn a_module_outside_the_checked_files_is_rejected() {
    let found = with_appended(SUPERVISOR, "#[path = \"../elsewhere.rs\"]\nmod elsewhere;");
    assert_rejected(&found, &["elsewhere"]);
    let found = with_appended(PIPES, "mod nested;");
    assert_rejected(&found, &["nested"]);
}

#[test]
fn a_stale_allowlist_entry_is_rejected() {
    let found = violations_after(SUPERVISOR, |source| {
        source.replace("fn retire_when_reaped", "fn retire_after_reap")
    });
    assert_rejected(&found, &["retire_when_reaped", "remove it"]);
}
