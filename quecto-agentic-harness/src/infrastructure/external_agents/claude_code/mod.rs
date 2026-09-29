//! The Claude Code CLI (`claude -p`) as an external agent (epic #2284):
//! its stream-json codec (#2285), and its process adapter with the
//! member's isolated environment (#2286).

mod arguments;
pub mod environment;
mod json_fields;
pub mod process;
mod result_json;
pub mod stream_json;
