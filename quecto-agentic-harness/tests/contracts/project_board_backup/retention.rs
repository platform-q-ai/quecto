use super::{board_backup::old_snapshot, fixture::*};
use quecto::application::project_board::ports::backup::BoardBackup;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;

#[test]
fn r3_gapped_represented_dates_and_midnight_are_not_rolling_336_hour_ttl() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let dates = [1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18, 20, 21, 24];
    for day in dates.into_iter().rev() {
        old_snapshot(&f.to, A, day, 9);
    }
    let midnight_old = f.to.join(format!("{A}-20261024T235959Z.sqlite"));
    std::fs::rename(
        f.to.join(format!("{A}-20261024T090000Z.sqlite")),
        &midnight_old,
    )
    .unwrap();
    integrity(&midnight_old);
    let mut request = f.request();
    request.at = Timestamp::parse("2026-10-25T00:00:01Z").unwrap();
    successful(f.adapter().backup(&request));
    let actual: Vec<_> = names(&f.to)
        .into_iter()
        .filter(|n| n.starts_with(A) && n.len() == 40 && n.ends_with(".sqlite"))
        .collect();
    let mut expected: Vec<_> = [5, 6, 8, 9, 11, 12, 14, 15, 17, 18, 20, 21]
        .into_iter()
        .map(|day| format!("{A}-202610{day:02}T090000Z.sqlite"))
        .collect();
    expected.push(format!("{A}-20261024T235959Z.sqlite"));
    expected.push(format!("{A}-20261025T000001Z.sqlite"));
    assert_eq!(
        actual, expected,
        "fourteen represented UTC dates survive gaps, including Oct5 older than 336h; two seconds over midnight still creates a fresh date"
    );
    assert!(
        f.to.join(format!("{A}-20261005T090000Z.sqlite")).exists(),
        "rolling TTL would wrongly remove this retained representative"
    );
}
