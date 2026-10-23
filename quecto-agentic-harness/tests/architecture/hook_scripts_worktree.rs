//! The hook installer, activator and checker agree on where hooks and the
//! `--no-verify` wrapper live, including in a linked worktree where `.git`
//! is a file rather than a directory (#2119).
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCRIPTS: [&str; 6] = [
    "install-hooks.sh",
    "activate-hooks.sh",
    "check-hooks-installed.sh",
    "git-wrapper.sh",
    "pre-commit.sh",
    "pre-push.sh",
];

/// Runs `bash -c script` in `dir` with git isolated from the user's config
/// and from any `GIT_*` state inherited from a hook that runs this test.
/// GNU timeout owns a process group and SIGKILLs it before output returns,
/// bounding descendants as well as the shell before a fixture can be dropped.
fn bash(dir: &Path, home: &Path, script: &str) -> Output {
    let mut command = Command::new("timeout");
    command.args(["-s", "KILL", "60s", "bash"]);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("bash runs")
}

fn assert_ok(what: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{what} failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository holding copies of the hook scripts, plus a linked worktree.
fn repository_with_worktree(tmp: &Path) -> (PathBuf, PathBuf) {
    let main = tmp.join("main");
    let scripts = main.join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    for name in SCRIPTS {
        std::fs::copy(Path::new("../scripts").join(name), scripts.join(name))
            .unwrap_or_else(|e| panic!("copy {name}: {e}"));
    }
    let setup = bash(
        &main,
        tmp,
        "git init -q -b master . && git add scripts && git commit -qm scripts \
         && git worktree add -q ../linked -b linked",
    );
    assert_ok("repository setup", &setup);
    (main, tmp.join("linked"))
}

fn install_activate_and_check(dir: &Path, home: &Path) {
    let installed = bash(dir, home, "bash scripts/install-hooks.sh");
    assert_ok("install-hooks.sh", &installed);
    let checked = bash(
        dir,
        home,
        "source scripts/activate-hooks.sh && bash scripts/check-hooks-installed.sh",
    );
    assert_ok("activate + check-hooks-installed.sh", &checked);
}

#[test]
fn hooks_install_and_verify_in_a_linked_worktree() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_, linked) = repository_with_worktree(tmp.path());
    assert!(
        linked.join(".git").is_file(),
        "a linked worktree's .git is a file"
    );

    install_activate_and_check(&linked, tmp.path());
}

#[test]
fn hooks_install_and_verify_in_the_main_checkout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    assert!(main.join(".git").is_dir());

    install_activate_and_check(&main, tmp.path());
}

#[test]
fn one_activation_covers_every_worktree_of_the_repository() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    let checked = bash(
        &main,
        tmp.path(),
        "bash scripts/install-hooks.sh >/dev/null && source scripts/activate-hooks.sh \
         && cd ../linked && bash scripts/check-hooks-installed.sh",
    );
    assert_ok(
        "check in the linked worktree after activating in main",
        &checked,
    );
}

#[test]
fn a_configured_hooks_path_is_refused_and_left_untouched() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    let shared = tmp.path().join("shared-hooks");
    std::fs::create_dir_all(&shared).expect("shared hooks dir");
    std::fs::write(shared.join("pre-merge-commit"), "#!/bin/sh\nexit 0\n").expect("user hook");
    let configured = bash(
        &main,
        tmp.path(),
        &format!("git config --global core.hooksPath {}", shared.display()),
    );
    assert_ok("configure core.hooksPath", &configured);

    let installed = bash(&main, tmp.path(), "bash scripts/install-hooks.sh");

    assert!(!installed.status.success(), "install must refuse");
    assert!(
        String::from_utf8_lossy(&installed.stderr).contains("core.hooksPath"),
        "the refusal names the setting: {}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let mut left: Vec<_> = std::fs::read_dir(&shared)
        .expect("read shared hooks")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    left.sort();
    assert_eq!(left, ["pre-merge-commit"], "the user's hooks are untouched");
}

#[test]
fn activating_outside_a_repository_fails_without_touching_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loose = tmp.path().join("loose");
    std::fs::create_dir_all(loose.join("scripts")).expect("scripts dir");
    for name in ["activate-hooks.sh", "git-wrapper.sh"] {
        std::fs::copy(
            Path::new("../scripts").join(name),
            loose.join("scripts").join(name),
        )
        .expect("copy script");
    }
    let sourced = bash(
        &loose,
        tmp.path(),
        "before=\"$PATH\"; source scripts/activate-hooks.sh; status=$?; \
         [ \"$PATH\" = \"$before\" ] && echo unchanged; exit $status",
    );
    assert!(!sourced.status.success(), "activation must report failure");
    let stdout = String::from_utf8_lossy(&sourced.stdout);
    assert!(stdout.contains("unchanged"), "PATH changed: {stdout}");
    assert!(!stdout.contains("activated"), "no false success: {stdout}");
}

#[test]
fn activation_works_with_cdpath_set() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    let checked = bash(
        &main,
        tmp.path(),
        "bash scripts/install-hooks.sh >/dev/null && export CDPATH=.:/tmp \
         && source scripts/activate-hooks.sh && bash scripts/check-hooks-installed.sh",
    );
    assert_ok("activate with CDPATH set", &checked);
}

#[test]
fn a_hooks_path_naming_the_repository_hooks_directory_is_accepted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    let configured = bash(&main, tmp.path(), "git config core.hooksPath .git/hooks");
    assert_ok("configure core.hooksPath", &configured);

    install_activate_and_check(&main, tmp.path());
}

#[test]
fn activation_without_the_wrapper_script_fails_without_touching_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    std::fs::remove_file(main.join("scripts/git-wrapper.sh")).expect("remove wrapper");
    let sourced = bash(
        &main,
        tmp.path(),
        "before=\"$PATH\"; source scripts/activate-hooks.sh; status=$?; \
         [ \"$PATH\" = \"$before\" ] && echo unchanged; exit $status",
    );
    assert!(!sourced.status.success(), "activation must report failure");
    assert!(String::from_utf8_lossy(&sourced.stdout).contains("unchanged"));
}

const LARGE_FILE_DIAGNOSTIC: &str = "new file imported file.rs exceeds 750 lines";

/// Exercise the real hook without installing hooks or creating fixture commits
/// via porcelain. Plumbing supplies HEAD and a valid incoming merge parent.
/// The index and worktree deliberately agree: this characterizes worktree line
/// counting, not staged-size enforcement (staged 751/worktree 1 remains a hole).
fn actual_pre_commit_for_staged_rust(lines: usize, imported: bool) -> Output {
    assert!(matches!(lines, 750 | 751));
    let tmp = tempfile::tempdir().expect("hook fixture tempdir");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("fixture repo");
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
         [lib]\npath = \"imported file.rs\"\n",
    )
    .expect("minimal real manifest");
    std::fs::write(repo.join("imported file.rs"), "// fixture\n".repeat(lines))
        .expect("formatted Rust fixture");
    let setup = bash(
        &repo,
        tmp.path(),
        &format!(
            r#"set -euo pipefail
file='imported file.rs'
git init -q -b hook-fixture .
empty=$(git mktree </dev/null)
base=$(printf 'base\n' | git commit-tree "$empty")
git update-ref refs/heads/hook-fixture "$base"
blob=$(git hash-object -w -- "$file")
if [ '{imported}' = true ]; then
    tree=$(printf '100644 blob %s\t%s\n' "$blob" "$file" | git mktree)
else
    tree=$empty
fi
parent=$(printf 'incoming\n' | git commit-tree "$tree" -p "$base")
printf '%s\n' "$parent" > .git/MERGE_HEAD
git add -- "$file"
test "$(git cat-file -t "$parent")" = commit
test "$(git rev-parse "$parent^")" = "$base"
test "$(cat .git/MERGE_HEAD)" = "$parent"
test "$(git cat-file -t "$blob")" = blob
test "$(git ls-files --stage -- "$file")" = "$(printf '100644 %s 0\t%s' "$blob" "$file")"
test "$(git diff --cached --name-status -- "$file")" = "$(printf 'A\t%s' "$file")"
test "$(git diff --cached --name-only | wc -l)" -eq 1
test "$(wc -l < "$file")" -eq {lines}
test "$(git cat-file blob ":0:$file" | wc -l)" -eq {lines}
if [ '{imported}' = true ]; then
    test "$(git ls-tree "$parent" -- "$file")" = "$(printf '100644 blob %s\t%s' "$blob" "$file")"
    test "$(git rev-parse "$parent:$file")" = "$(git rev-parse ":0:$file")"
else
    test "$(git ls-tree "$parent" -- "$file")" = ''
fi
cargo fmt --all -- --check
printf 'fixture verified: imported={imported}; lines={lines}; valid merge parent; regular stage-0 blob; staged A path with spaces; real cargo fmt passed\n'
"#
        ),
    );
    assert_ok("actual-hook fixture invariants and real cargo fmt", &setup);
    print!("{}", String::from_utf8_lossy(&setup.stdout));
    let hook = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/pre-commit.sh")
        .canonicalize()
        .expect("absolute actual hook path");
    assert!(hook.is_absolute() && hook.is_file());
    let quoted_hook = hook
        .to_str()
        .expect("hook path UTF-8")
        .replace('\'', "'\\''");
    let output = bash(&repo, tmp.path(), &format!("bash '{quoted_hook}'"));
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "hook must finish normally, not time out: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    tmp.close()
        .expect("remove fixture after bounded child exits");
    println!("fixture cleaned: imported={imported}; lines={lines}");
    output
}

fn assert_large_file_rejection(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "hook must reject normally");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(LARGE_FILE_DIAGNOSTIC),
        "rejection must specifically identify the line cap\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        "actual size rejection:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn imported_identical_large_staged_file_passes_actual_pre_commit() {
    let output = actual_pre_commit_for_staged_rust(751, true);
    // On the baseline, isolate the intended size failure before asserting the
    // desired success. A future merge-aware hook may legitimately return zero.
    if output.status.code() == Some(1) {
        assert_large_file_rejection(&output);
    }
    assert_ok(
        "identical 751-line import from MERGE_HEAD must pass",
        &output,
    );
}

#[test]
fn genuinely_new_file_size_boundary_is_preserved_by_actual_pre_commit() {
    let oversized = actual_pre_commit_for_staged_rust(751, false);
    assert_large_file_rejection(&oversized);
    let boundary = actual_pre_commit_for_staged_rust(750, false);
    assert_ok("genuinely new 750-line file must pass", &boundary);
    assert!(String::from_utf8_lossy(&boundary.stdout).contains("Pre-commit passed."));
}

/// Each case uses actual objects and the actual hook (including real rustfmt).
/// TempDir also removes fixtures on assertion unwinding; bash kills descendants
/// before returning on timeout. BDD paths are not staged: production BDD is intact.
#[test]
fn merge_cap_policy_and_unrelated_guards_use_actual_hook() {
    let cases = [
        ("no-head", false, "exceeds 750 lines"),
        ("empty", false, "exceeds 750 lines"),
        ("blank", false, "exceeds 750 lines"),
        ("nul", false, "exceeds 750 lines"),
        ("invalid", false, "exceeds 750 lines"),
        ("abbreviated", false, "exceeds 750 lines"),
        ("ref-name", false, "exceeds 750 lines"),
        ("blob", false, "exceeds 750 lines"),
        ("tag", false, "exceeds 750 lines"),
        ("unavailable", false, "exceeds 750 lines"),
        ("absent", false, "exceeds 750 lines"),
        ("modified", false, "exceeds 750 lines"),
        ("new-750", true, "Pre-commit passed."),
        ("staged-short", false, "exceeds 750 lines"),
        ("worktree-short", true, "Pre-commit passed."),
        ("worktree-different", true, "Pre-commit passed."),
        ("mode", true, "Pre-commit passed."),
        ("index-executable", true, "Pre-commit passed."),
        ("symlink-parent", false, "exceeds 750 lines"),
        ("symlink-index", false, "exceeds 750 lines"),
        ("gitlink-parent", false, "exceeds 750 lines"),
        // U paths are outside the existing ACMR loop, not size-exempt.
        ("unmerged", true, "Pre-commit passed (nothing staged)."),
        ("multi-match", true, "Pre-commit passed."),
        ("multi-miss", false, "exceeds 750 lines"),
        ("match-invalid", false, "exceeds 750 lines"),
        ("invalid-match", false, "exceeds 750 lines"),
        ("duplicate", true, "Pre-commit passed."),
        ("unterminated", true, "Pre-commit passed."),
        ("literal", true, "Pre-commit passed."),
        ("existing-M", true, "Pre-commit passed."),
        ("linked", true, "Pre-commit passed."),
        ("branch", false, "Direct commits to main"),
        ("marker", false, "forbidden work marker"),
        ("unsafe", false, "unsafe block without adjacent SAFETY"),
        ("fmt", false, "Diff in"),
    ];
    let hook = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/pre-commit.sh")
        .canonicalize()
        .expect("actual hook");
    let quoted_hook = hook.to_str().expect("UTF-8 hook").replace('\'', "'\\''");
    for (case, success, diagnostic) in cases {
        let tmp = tempfile::tempdir().expect("policy fixture");
        let setup = bash(
            tmp.path(),
            tmp.path(),
            &format!(
                r#"set -euo pipefail
case='{case}'
mkdir repo
cd repo
git init -q -b fixture .
empty=$(git mktree </dev/null)
base=$(printf 'base\n' | git commit-tree "$empty")
git update-ref refs/heads/fixture "$base"
if [ "$case" = linked ]; then
    git worktree add -q ../linked -b linked
    cd ../linked
fi
file='imported file.rs'
if [ "$case" = literal ]; then file='imported [x]* file.rs'; fi
printf '[package]\nname="policy-fixture"\nversion="0.1.0"\nedition="2021"\n[lib]\npath="%s"\n' "$file" > Cargo.toml
for ((i=0;i<751;i++)); do printf '// fixture\n'; done > "$file"
case "$case" in
    new-750) head -n 750 "$file" > short; mv short "$file" ;;
    marker) printf '// FIX%s\n' 'ME' >> "$file" ;;
    unsafe) printf 'pub fn check() {{\n    unsafe {{}}\n}}\n' >> "$file" ;; # SAFETY: writes adversarial fixture text only; no unsafe code executes.
    fmt) printf 'pub fn badly_formatted( ){{}}\n' >> "$file" ;;
esac
blob=$(git hash-object -w -- "$file")
other=$(printf '// other\n' | git hash-object -w --stdin)
mode=100644; kind=blob; incoming=$blob
case "$case" in
    mode) mode=100755 ;;
    symlink-parent) mode=120000 ;;
    gitlink-parent) mode=160000; kind=commit; incoming=$base ;;
esac
tree=$(printf '%s %s %s\t%s\0' "$mode" "$kind" "$incoming" "$file" | git mktree -z)
parent=$(printf 'incoming\n' | git commit-tree "$tree")
missing=$(printf 'absent\n' | git commit-tree "$empty")
merge_head=$(git rev-parse --git-path MERGE_HEAD)
printf '%s\n' "$parent" > "$merge_head"
case "$case" in
    no-head) rm "$merge_head" ;;
    empty) : > "$merge_head" ;;
    blank) printf '\n' > "$merge_head" ;;
    nul) printf '%s\0\n' "$parent" > "$merge_head" ;;
    invalid) printf 'not-an-object\n' > "$merge_head" ;;
    abbreviated) printf '%.12s\n' "$parent" > "$merge_head" ;;
    ref-name) printf 'refs/heads/fixture\n' > "$merge_head" ;;
    blob) printf '%s\n' "$blob" > "$merge_head" ;;
    tag) tag=$(printf 'object %s\ntype commit\ntag policy\ntagger t <t@example.com> 1 +0000\n\ntag\n' "$parent" | git mktag); printf '%s\n' "$tag" > "$merge_head" ;;
    unavailable) printf '%040d\n' 0 > "$merge_head" ;;
    absent|new-750|worktree-short) printf '%s\n' "$missing" > "$merge_head" ;;
    multi-match) printf '%s\n%s\n' "$missing" "$parent" > "$merge_head" ;;
    multi-miss) printf '%s\n%s\n' "$missing" "$missing" > "$merge_head" ;;
    match-invalid) printf '%s\nbad\n' "$parent" > "$merge_head" ;;
    invalid-match) printf 'bad\n%s\n' "$parent" > "$merge_head" ;;
    duplicate) printf '%s\n%s\n' "$parent" "$parent" > "$merge_head" ;;
    unterminated) printf '%s' "$parent" > "$merge_head" ;;
esac
git add -- ":(literal)$file"
case "$case" in
    modified|staged-short) git update-index --cacheinfo "100644,$other,$file" ;;
    worktree-short) printf '// short\n' > "$file" ;;
    worktree-different) printf '// differing worktree\n' >> "$file" ;;
    index-executable) git update-index --cacheinfo "100755,$blob,$file" ;;
    symlink-index) git update-index --cacheinfo "120000,$blob,$file" ;;
    unmerged) git update-index --force-remove -- "$file"; printf '100644 %s 1\t%s\n100644 %s 2\t%s\n' "$blob" "$file" "$other" "$file" | git update-index --index-info ;;
    existing-M) oldtree=$(printf '100644 blob %s\t%s\0' "$other" "$file" | git mktree -z); old=$(printf 'old\n' | git commit-tree "$oldtree"); git update-ref HEAD "$old" ;;
    branch) git symbolic-ref HEAD refs/heads/main; git update-ref HEAD "$base" ;;
esac
status=$(git diff --cached --name-status -- ":(literal)$file")
case "$case" in
    existing-M) test "$status" = "$(printf 'M\t%s' "$file")" ;;
    unmerged) test "$(git ls-files -u | wc -l)" -eq 2 ;;
    *) test "$status" = "$(printf 'A\t%s' "$file")" ;;
esac
test "$(git cat-file -t "$parent")" = commit
if [ "$case" = fmt ]; then :; else cargo fmt --all -- --check; fi
printf 'fixture verified: %s; real objects and fmt\n' "$case"
"#
            ),
        );
        assert_ok(&format!("setup {case}"), &setup);
        let directory = if case == "linked" { "linked" } else { "repo" };
        let output = bash(
            &tmp.path().join(directory),
            tmp.path(),
            &format!("bash '{quoted_hook}'"),
        );
        tmp.close().expect("cleanup after bounded actual hook");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.status.code(),
            Some(i32::from(!success)),
            "{case}: {text}"
        );
        assert!(text.contains(diagnostic), "{case}: {text}");
        if matches!(case, "branch" | "marker" | "unsafe" | "fmt") {
            assert!(!text.contains("exceeds 750 lines"), "size exempt: {text}");
        }
        println!(
            "actual hook case={case}: exit={}; diagnostic={diagnostic}; fixture cleaned",
            output.status
        );
    }
}
