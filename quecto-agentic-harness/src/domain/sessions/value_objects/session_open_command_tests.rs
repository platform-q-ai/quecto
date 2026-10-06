use super::*;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[test]
fn the_command_opens_quecto_there_and_names_no_session_flag() {
    // `quecto-tui` takes no `-s`: the resume is a second step, typed inside.
    assert_eq!(
        open_there_command(Path::new("/work/alpha")).as_deref(),
        Some("cd '/work/alpha' && quecto-tui")
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
            open_there_command(Path::new(dir)),
            Some(format!("cd {quoted} && quecto-tui")),
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
        assert_eq!(open_there_command(Path::new(dir)), None, "{dir:?}");
        assert_eq!(cd_there_command(Path::new(dir)), None, "{dir:?}");
    }
    let not_utf8 = Path::new(OsStr::from_bytes(b"/w/caf\xe9"));
    assert_eq!(open_there_command(not_utf8), None);
    assert_eq!(resume_step("cli:al\u{202e}pha"), None);
    assert_eq!(resume_step("two\nlines"), None);
}
