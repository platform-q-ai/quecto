use super::*;

#[test]
fn the_flag_is_lowered_until_announced_and_stays_raised() {
    let flag = OwnerExitFlag::new();
    assert!(!flag.announced());
    flag.announce();
    assert!(flag.announced());
    flag.announce();
    assert!(flag.announced());
}
