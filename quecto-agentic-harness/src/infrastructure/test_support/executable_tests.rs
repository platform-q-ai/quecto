use super::*;
use std::process::Command;

/// Threads forking alongside the stress test, and the execs it makes: few
/// forkers keep the load it adds to the parallel suite small, while enough
/// execs still catch an in-process writer (the `std::fs::write` mutation)
/// every run.
const FORKERS: usize = 2;
const EXECS: usize = 300;

fn run(path: &Path) -> std::io::Result<String> {
    let output = Command::new(path).output()?;
    assert!(
        output.status.success(),
        "{} ran: {output:?}",
        path.display()
    );
    Ok(String::from_utf8(output.stdout).expect("utf-8 stdout"))
}

#[test]
fn the_executable_holds_the_bytes_with_the_executable_mode_and_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake-tool");
    write_executable(&path, "#!/bin/sh\necho ran \"$0\"\n");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        EXECUTABLE_MODE
    );
    assert_eq!(run(&path).unwrap(), format!("ran {}\n", path.display()));
}

#[test]
fn a_rewrite_replaces_the_executable_and_leaves_no_staging_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake-tool");
    write_executable(&path, "#!/bin/sh\necho first\n");
    write_executable(&path, b"#!/bin/sh\necho second\n");
    assert_eq!(run(&path).unwrap(), "second\n");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("fake-tool")]);
}

#[test]
fn a_path_with_a_quote_or_space_is_written_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("it's a \"tool\" $HOME");
    write_executable(&path, "#!/bin/sh\necho ok\n");
    assert_eq!(run(&path).unwrap(), "ok\n");
}

/// The #2232 race, reproduced: other threads fork continuously while this one
/// writes and at once execs fresh scripts. With `std::fs::write` a few percent
/// of these execs fail with `ETXTBSY`; through the helper none may.
#[test]
fn an_exec_right_after_the_write_never_meets_a_busy_text_file_while_others_fork() {
    let dir = tempfile::tempdir().unwrap();
    let busy = while_forking(FORKERS, || {
        (0..EXECS)
            .filter(|index| {
                let path = dir.path().join(format!("script-{index}"));
                write_executable(&path, "#!/bin/sh\nexit 0\n");
                exec_is_busy(&path)
            })
            .count()
    });
    assert_eq!(busy, 0, "execs refused with ETXTBSY");
}

#[test]
#[should_panic(expected = "names a file")]
fn a_path_without_a_file_name_is_refused() {
    write_executable(Path::new("/"), "#!/bin/sh\n");
}

#[test]
#[should_panic(expected = "cannot create")]
fn a_directory_that_does_not_exist_is_reported_in_the_writers_words() {
    let dir = tempfile::tempdir().unwrap();
    write_executable(&dir.path().join("missing/fake-tool"), "#!/bin/sh\n");
}

/// #2232 review: a panic in the stressed work still stops and joins every
/// forker, so none outlives its test.
#[test]
fn a_panic_in_the_work_stops_and_joins_the_forkers() {
    let forkers = Forkers::start(2);
    let stop = forkers.stop.clone();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _forkers = forkers;
        panic!("the stressed work failed");
    }));
    assert!(unwound.is_err());
    assert!(
        stop.load(Ordering::Relaxed),
        "the forkers were told to stop"
    );
    assert_eq!(
        std::sync::Arc::strong_count(&stop),
        1,
        "every forker thread was joined"
    );
}
