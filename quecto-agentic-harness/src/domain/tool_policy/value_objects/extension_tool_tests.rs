use super::*;

#[test]
fn a_timeout_is_whole_seconds_from_one_to_six_hundred() {
    for seconds in [1, 30, 600] {
        let timeout = ExtensionToolTimeout::from_seconds(seconds).expect("allowed");
        assert_eq!(timeout.seconds(), seconds);
        assert_eq!(timeout.duration(), std::time::Duration::from_secs(seconds));
    }
    for seconds in [0, 601, u64::MAX] {
        assert_eq!(ExtensionToolTimeout::from_seconds(seconds), None);
    }
}

#[test]
fn the_default_is_thirty_seconds_and_allowed() {
    let default = ExtensionToolTimeout::DEFAULT;
    assert_eq!(default.seconds(), 30);
    assert_eq!(ExtensionToolTimeout::from_seconds(30), Some(default));
}
