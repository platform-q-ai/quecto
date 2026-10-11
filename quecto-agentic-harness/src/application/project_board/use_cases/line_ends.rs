//! Text as the board keeps it: CRLF line ends turned to LF once, here, for
//! every front end (#2477), before the task's schema reads it (the schema
//! refuses `\r`).
pub(super) fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}
