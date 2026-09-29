//! #2192 / ADR-0029: release builds unwind so a panicking tool call can be
//! contained, and every binary keeps fail-fast behaviour by installing a
//! panic hook first thing in `main`. A new binary without one would turn
//! every panic in a spawned task into a silent `JoinError`. A tool call's
//! carried work is spawned and joined only through `call_work`, so its panic
//! always resumes in the call instead of being read as "no output".
use std::path::{Path, PathBuf};

/// The hook each binary target installs as the first statement of `main`:
/// the harness's own, else the shared abort-on-panic hook.
fn expected_install(binary: &str) -> &'static str {
    match binary {
        "quecto" => "quecto::interface::panic_hook::install();",
        _ => "quecto_fail_fast::abort_on_panic();",
    }
}

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

/// Every binary target of the workspace, as cargo sees it: (name, main).
fn binaries() -> Vec<(String, PathBuf)> {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(workspace())
        .output()
        .expect("cargo metadata runs");
    assert!(output.status.success(), "{output:?}");
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut binaries: Vec<(String, PathBuf)> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|package| package["targets"].as_array().unwrap().iter())
        .filter(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .map(|target| {
            (
                target["name"].as_str().unwrap().to_string(),
                PathBuf::from(target["src_path"].as_str().unwrap()),
            )
        })
        .collect();
    binaries.sort();
    binaries
}

#[test]
fn the_release_profile_unwinds() {
    let manifest = std::fs::read_to_string(workspace().join("Cargo.toml")).unwrap();
    let release = manifest
        .split("[profile.release]")
        .nth(1)
        .expect("a release profile");
    let section = release.split("\n[").next().unwrap();
    let strategies: Vec<&str> = section
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("panic"))
        .collect();
    assert_eq!(strategies, ["panic = \"unwind\""], "{section}");
}

#[test]
fn every_binary_installs_its_panic_hook_first() {
    let binaries = binaries();
    let names: Vec<&str> = binaries.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        names.contains(&"quecto") && names.len() >= 5,
        "cargo lists the workspace binaries: {names:?}"
    );
    for (name, main) in &binaries {
        let source = std::fs::read_to_string(main).unwrap();
        let body = source
            .split("fn main()")
            .nth(1)
            .unwrap_or_else(|| panic!("{} has a main", main.display()));
        let first_statement = body
            .split_once('{')
            .map(|(_, rest)| rest)
            .unwrap()
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap();
        assert_eq!(first_statement, expected_install(name), "{name}");
    }
}

/// Production sources under `dir`, as (path, text), test files excluded.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            match (
                path.is_dir(),
                name.ends_with(".rs"),
                name.ends_with("_tests.rs"),
            ) {
                (true, _, _) => stack.push(path),
                (false, true, false) => {
                    let text = std::fs::read_to_string(&path).unwrap();
                    found.push((path, text));
                }
                (false, _, _) => {}
            }
        }
    }
    found
}

#[test]
fn carried_work_is_joined() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let carriers: Vec<PathBuf> = sources(&crate_root.join("src/infrastructure"))
        .into_iter()
        .chain(sources(&crate_root.join("src/interface")))
        .chain(sources(&crate_root.join("src/composition")))
        .filter(|(_, text)| {
            text.contains("tool_panic_scope::carry")
                || text.contains("carry(")
                || text.contains("carry_future(")
        })
        .map(|(path, _)| path.strip_prefix(crate_root).unwrap().to_path_buf())
        .collect();
    assert_eq!(
        carriers,
        [PathBuf::from("src/infrastructure/tools/call_work.rs")],
        "only call_work carries a call's scope, and it joins what it spawns"
    );
    let call_work =
        std::fs::read_to_string(crate_root.join("src/infrastructure/tools/call_work.rs")).unwrap();
    assert!(
        call_work.contains("if error.is_panic() =>")
            && call_work.contains("std::panic::resume_unwind(error.into_panic())"),
        "joining carried work resumes its panic"
    );
    // Every spawn form call_work uses: the tokio tasks are joined through
    // `join_carried`; the one std thread (`std_thread_in_call`, the `find`
    // owner) is the detached exception, whose panic the agent loop's
    // backstop fails the call on (`execute_contained`).
    let spawns: Vec<&str> = call_work
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter(|line| {
            line.contains("tokio::task::spawn_blocking(")
                || line.contains("tokio::task::spawn(")
                || line.contains("tokio::spawn(")
                || line.contains(".spawn_blocking(")
                || line.contains("builder.spawn(")
        })
        .collect();
    let (threads, tasks): (Vec<&str>, Vec<&str>) = spawns
        .iter()
        .partition(|line| line.contains("builder.spawn("));
    assert_eq!(
        (tasks.len(), threads.len()),
        (3, 1),
        "call_work spawns three kinds of task and one std thread: {spawns:?}"
    );
    for task in &tasks {
        assert!(
            task.contains("join_carried("),
            "every task call_work spawns is joined through join_carried: {task}"
        );
    }
    assert_eq!(
        threads,
        ["builder.spawn(carry(job))"],
        "the one std thread is carried and detached; the agent loop's backstop fails its call"
    );
    assert!(
        call_work.contains("detached") && call_work.contains("execute_contained"),
        "call_work names the detached-thread exception and its backstop"
    );
}

/// The functions that may catch a panic by hand, by file, and why.
/// Anywhere else a tool call's code handles a panic through
/// `tool_panic_scope::catch_in_call`, which ends the contained unwind and
/// forgets the handled panic; a raw catch would leave both behind (#2192
/// F1). Checked per function (#2192 review): a whole file is never
/// sanctioned.
const SANCTIONED_CATCHES: &[(&str, &str, &str)] = &[
    (
        "src/application/tool_panic_scope.rs",
        "catch_in_call",
        "the sanctioned catch itself",
    ),
    (
        "src/application/agent_loop_tool_exec.rs",
        "execute_contained",
        "the containment every tool call runs in",
    ),
    (
        "src/application/context.rs",
        "poison_context_gauge_lock_for_test",
        "a #[cfg(test)] helper that poisons a lock",
    ),
    (
        "src/infrastructure/persistence/swarm_board/meter.rs",
        "busy_callback",
        "SQLite's busy handler (#2303): an extern \"C\" callback, which a panic must never unwind out of into SQLite; it gives up the wait instead",
    ),
];

/// The innermost functions of `source` whose bodies name a panic catch
/// (`catch_unwind`, `CatchUnwind`); `<outside a function>` for a mention
/// in no function at all.
fn catching_functions(source: &str) -> Vec<String> {
    use quote::ToTokens;
    use syn::visit::Visit;
    fn catches(tokens: &proc_macro2::TokenStream) -> bool {
        let text = tokens.to_string();
        text.contains("catch_unwind") || text.contains("CatchUnwind")
    }
    #[derive(Default)]
    struct Finder(Vec<String>);
    impl Finder {
        fn function(
            &mut self,
            name: &syn::Ident,
            block: &syn::Block,
            visit: impl FnOnce(&mut Self),
        ) {
            let before = self.0.len();
            visit(self);
            if self.0.len() == before && catches(&block.to_token_stream()) {
                self.0.push(name.to_string());
            }
        }
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_item_fn(&mut self, item: &'a syn::ItemFn) {
            self.function(&item.sig.ident, &item.block, |finder| {
                syn::visit::visit_item_fn(finder, item)
            });
        }
        fn visit_impl_item_fn(&mut self, item: &'a syn::ImplItemFn) {
            self.function(&item.sig.ident, &item.block, |finder| {
                syn::visit::visit_impl_item_fn(finder, item)
            });
        }
    }
    let file = syn::parse_file(source).expect("the source parses");
    let mut finder = Finder::default();
    finder.visit_file(&file);
    // Every mention is inside a found function, or reported as outside.
    let mut outside = file.clone();
    outside.items.retain(|item| {
        !matches!(
            item,
            syn::Item::Fn(_) | syn::Item::Impl(_) | syn::Item::Mod(_)
        )
    });
    let items: proc_macro2::TokenStream = outside
        .items
        .iter()
        .map(ToTokens::to_token_stream)
        .collect();
    if catches(&items) {
        finder.0.push("<outside a function>".to_string());
    }
    finder.0
}

#[test]
fn panics_are_caught_in_the_sanctioned_way() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut catchers: Vec<(String, String)> = sources(&crate_root.join("src"))
        .into_iter()
        .flat_map(|(path, text)| {
            let file = path
                .strip_prefix(crate_root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            catching_functions(&text)
                .into_iter()
                .map(move |function| (file.clone(), function))
        })
        .collect();
    catchers.sort();
    let mut sanctioned: Vec<(String, String)> = SANCTIONED_CATCHES
        .iter()
        .map(|(path, function, _)| ((*path).to_string(), (*function).to_string()))
        .collect();
    sanctioned.sort();
    assert_eq!(
        catchers, sanctioned,
        "a panic is caught only through tool_panic_scope::catch_in_call"
    );
}

#[test]
fn a_catch_is_sanctioned_per_function_not_per_file() {
    let source = r#"
        fn catch_in_call() { let _ = std::panic::catch_unwind(|| ()); }
        fn another() { let _ = std::panic::catch_unwind(|| ()); }
        fn clean() {}
        impl X { fn method(&self) { let _ = f.catch_unwind(); } }
        static S: fn() = || { let _ = std::panic::catch_unwind(|| ()); };
    "#;
    assert_eq!(
        catching_functions(source),
        ["catch_in_call", "another", "method", "<outside a function>"]
    );
}
