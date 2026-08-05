#![deny(dead_code)]
#![deny(unused_imports)]

pub mod application;
pub use application::subagent_launch as subagent_launch_app;
pub mod domain;
pub mod infrastructure;
pub mod interface;
