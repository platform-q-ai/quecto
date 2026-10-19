//! The auth capability's clock (#1839): this machine's time, to the second.

use crate::application::auth::ports::AuthClock;

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemAuthClock;

impl AuthClock for SystemAuthClock {
    fn now_unix_secs(&self) -> i64 {
        super::unix_timestamp_secs()
    }
}
