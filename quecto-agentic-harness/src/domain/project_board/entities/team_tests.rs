//! A run's deadline, in seconds from Start: the team's own, or a day when
//! it names none or the plan names no team. Start's brief and a listing's
//! summary both read it here.
use super::{
    DEFAULT_DEADLINE_SECONDS, MAX_DEADLINE_SECONDS, MIN_DEADLINE_SECONDS, Team,
    run_deadline_seconds,
};

fn team(deadline_seconds: Option<u32>) -> Team {
    Team {
        roles: vec![],
        deadline_seconds,
        image: None,
    }
}

#[test]
fn a_team_naming_a_deadline_gives_its_own() {
    for seconds in [MIN_DEADLINE_SECONDS, 3_600, MAX_DEADLINE_SECONDS] {
        assert_eq!(team(Some(seconds)).deadline_seconds_or_default(), seconds);
        assert_eq!(run_deadline_seconds(Some(&team(Some(seconds)))), seconds);
    }
}

#[test]
fn no_deadline_or_no_team_is_a_day() {
    assert_eq!(DEFAULT_DEADLINE_SECONDS, 24 * 60 * 60);
    assert_eq!(
        team(None).deadline_seconds_or_default(),
        DEFAULT_DEADLINE_SECONDS
    );
    assert_eq!(
        run_deadline_seconds(Some(&team(None))),
        DEFAULT_DEADLINE_SECONDS
    );
    assert_eq!(run_deadline_seconds(None), DEFAULT_DEADLINE_SECONDS);
}
