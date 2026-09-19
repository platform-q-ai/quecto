//! Time as an INPUT of the `/resume` picker's deadlines (#2010 R3-T1): the
//! owed Enter and the search flight read "now" from here, never from the
//! wall. Production follows tokio's clock; a headless harness holds a
//! manual one, so a scenario says how much time passed instead of spending it.
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct Clock(Option<Arc<Mutex<Instant>>>);

impl Clock {
    /// A clock that stands still until [`Clock::advance`] moves it.
    pub fn manual() -> Self {
        Self(Some(Arc::new(Mutex::new(Instant::now()))))
    }
    pub fn now(&self) -> Instant {
        match &self.0 {
            Some(at) => *at.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            None => Instant::now(),
        }
    }
    /// Move a manual clock forward; every clone sees it. The system clock
    /// is nobody's to move: nothing happens.
    pub fn advance(&self, by: Duration) {
        if let Some(at) = &self.0 {
            *at.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) += by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_clock_moves_only_when_advanced_and_its_clones_agree() {
        let clock = Clock::manual();
        let seen = clock.clone();
        let start = clock.now();
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(seen.now(), start, "the wall moved, this clock did not");
        clock.advance(Duration::from_secs(6));
        assert_eq!(seen.now(), start + Duration::from_secs(6));
    }

    #[tokio::test(start_paused = true)]
    async fn the_system_clock_is_tokios_and_cannot_be_advanced_by_hand() {
        let clock = Clock::default();
        let start = clock.now();
        clock.advance(Duration::from_secs(6));
        assert_eq!(clock.now(), start);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(clock.now(), start + Duration::from_secs(1));
    }
}
