//! The shell command that opens quecto in a session's folder, resuming it (#2045, #2615).

/// `cd '<dir>' && quecto-tui --resume='<key>'`, or `None` if either would not read as it runs.
pub fn open_there_command(dir: &std::path::Path, session: &str) -> Option<String> {
    let cd = cd_there_command(dir).filter(|_| reads_as_it_runs(session))?;
    let key = shell_quote(session);
    Some(format!("{cd} && quecto-tui --resume={key}"))
}

/// `cd '<dir>'` — or `None` when the command would not read the way it runs:
/// a path that is no UTF-8, or one holding a terminal control or an invisible
/// reordering/format character. A client then shows the folder and no command.
pub fn cd_there_command(dir: &std::path::Path) -> Option<String> {
    let raw = dir.to_str().filter(|raw| reads_as_it_runs(raw))?;
    Some(format!("cd {}", shell_quote(raw)))
}

/// What to type in quecto, once open there, to resume `session`.
pub fn resume_step(session: &str) -> Option<String> {
    reads_as_it_runs(session).then(|| format!("/resume {session}"))
}

/// One POSIX shell word: single quotes make every character literal; a quote
/// itself is closed, escaped and reopened.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Every character is one a terminal shows where it stands: no control, no
/// Bidi_Control, zero-width, separator, soft hyphen, BOM or tag character —
/// the set the socket's `safe_display` replaces.
fn reads_as_it_runs(text: &str) -> bool {
    !text.chars().any(|ch| {
        ch.is_control()
            || matches!(ch,
                '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
                | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
    })
}

#[cfg(test)]
#[path = "session_open_command_tests.rs"]
mod tests;
