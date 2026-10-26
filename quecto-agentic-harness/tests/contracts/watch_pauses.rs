//! Contract for [`WatchPauses`] (epic #2705 B10), on the thread's sleep: a
//! pause lasts at least as long as asked.
use std::time::{Duration, Instant};

use quecto::application::project_board::ports::feed::WatchPauses;
use quecto::infrastructure::http::board_watch::ThreadPauses;

#[test]
fn a_pause_lasts_at_least_as_long_as_asked() {
    let began = Instant::now();
    ThreadPauses.pause(Duration::from_millis(30));
    assert!(began.elapsed() >= Duration::from_millis(30));
}
