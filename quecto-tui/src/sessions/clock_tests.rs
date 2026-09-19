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
