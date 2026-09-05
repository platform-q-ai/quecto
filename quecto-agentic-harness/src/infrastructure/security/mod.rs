pub(crate) mod ansi_c;
pub(crate) mod denylist;
pub(crate) mod legacy_scan;
pub mod sandbox;
pub(crate) mod shell_ast;
pub(crate) mod shell_parse;

#[cfg(test)]
#[path = "sandbox_escape_tests.rs"]
mod sandbox_escape_tests;

#[cfg(test)]
#[path = "shell_parse_tests.rs"]
mod shell_parse_tests;

#[cfg(test)]
#[path = "denylist_tests.rs"]
mod denylist_tests;
