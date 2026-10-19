//! The current time as the auth use cases read it (#1839).

pub trait AuthClock: Send + Sync {
    /// Seconds since the Unix epoch.
    fn now_unix_secs(&self) -> i64;
}
