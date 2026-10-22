use super::*;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[test]
fn the_command_opens_quecto_there_resuming_the_session() {
    // #2615: `quecto-tui --resume` resumes it there; `/resume` is the same
    // typed inside.
    assert_eq!(
        open_there_command(Path::new("/work/alpha"), "cli:alpha").as_deref(),
        Some("cd '/work/alpha' && quecto-tui --resume='cli:alpha'")
    );
    assert_eq!(
        cd_there_command(Path::new("/work/alpha")).as_deref(),
        Some("cd '/work/alpha'")
    );
    assert_eq!(
        resume_step("cli:alpha").as_deref(),
        Some("/resume cli:alpha")
    );
}

#[test]
fn shell_metacharacters_stay_inside_one_quoted_word() {
    for (dir, quoted) in [
        ("/w/a b", "'/w/a b'"),
        ("/w/it's", "'/w/it'\\''s'"),
        ("/w/$(touch pwned)", "'/w/$(touch pwned)'"),
        ("/w/`id`;rm -rf ~ && x", "'/w/`id`;rm -rf ~ && x'"),
        ("/w/\"q\" | tee", "'/w/\"q\" | tee'"),
        ("-rf", "'-rf'"),
        (
            "/w/caf\u{e9}/\u{65e5}\u{672c}",
            "'/w/caf\u{e9}/\u{65e5}\u{672c}'",
        ),
    ] {
        assert_eq!(shell_quote(dir), quoted, "{dir}");
        assert_eq!(
            open_there_command(Path::new(dir), "cli:x"),
            Some(format!("cd {quoted} && quecto-tui --resume='cli:x'")),
            "{dir}"
        );
    }
}

#[test]
fn a_path_that_would_not_read_the_way_it_runs_has_no_command() {
    for dir in [
        "/w/clear\u{1b}[2Jscreen", // terminal control
        "/w/two\nlines",
        "/w/tab\there",
        "/w/rtl\u{202e}gnp.exe", // bidi override
        "/w/zero\u{200b}width",
        "/w/soft\u{ad}hyphen",
        "/w/bom\u{feff}",
        "/w/tag\u{e0041}",
    ] {
        assert_eq!(open_there_command(Path::new(dir), "cli:x"), None, "{dir:?}");
        assert_eq!(cd_there_command(Path::new(dir)), None, "{dir:?}");
    }
    let not_utf8 = Path::new(OsStr::from_bytes(b"/w/caf\xe9"));
    assert_eq!(open_there_command(not_utf8, "cli:x"), None);
    assert_eq!(resume_step("cli:al\u{202e}pha"), None);
    assert_eq!(resume_step("two\nlines"), None);
}

/// #2615: the command opens quecto there resuming the session; a key that
/// would not read the way it runs gets no command.
#[test]
fn the_command_resumes_the_session_there() {
    assert_eq!(
        open_there_command(Path::new("/work/it's"), "cli:foreign").as_deref(),
        Some("cd '/work/it'\\''s' && quecto-tui --resume='cli:foreign'")
    );
    assert_eq!(
        open_there_command(Path::new("/work"), "cli:a\u{202e}b"),
        None
    );
    assert_eq!(
        open_there_command(Path::new("/work/a\u{1b}b"), "cli:x"),
        None
    );
}

/// #2615 review round 3: the one-word `--resume=` form carries a key that
/// starts with `-`.
#[test]
fn a_key_starting_with_a_dash_stays_the_key() {
    assert_eq!(
        open_there_command(Path::new("/work"), "--legacy").as_deref(),
        Some("cd '/work' && quecto-tui --resume='--legacy'")
    );
}
