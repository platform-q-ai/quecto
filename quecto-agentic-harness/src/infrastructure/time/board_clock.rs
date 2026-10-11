//! The project board's clock (#2476): this machine's time, to the second.
//! It has no outside reference yet: the board store will supply one (the
//! remote's time) when it can read it.
use std::time::SystemTime;

use crate::application::project_board::ports::BoardClock;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBoardClock;

impl BoardClock for SystemBoardClock {
    fn now(&self) -> Timestamp {
        let text = humantime::format_rfc3339_seconds(SystemTime::now()).to_string();
        Timestamp::parse(&text).expect("the system clock reads a time between 1970 and 9999")
    }

    fn reference(&self) -> Option<Timestamp> {
        None
    }
}
