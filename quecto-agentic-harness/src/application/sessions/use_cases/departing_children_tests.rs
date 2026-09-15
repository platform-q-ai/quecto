use super::super::start_fresh_rig::{FreshOptions, build_fresh_rig};
use crate::application::sessions::dto::{
    FleetSettled, FleetSettlementOutcome, SessionTransition, SessionTransitionRefused,
};
use crate::domain::ids::AgentUuid;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn a_settled_fleet_reports_the_rows_it_removed() {
    let rig = build_fresh_rig(FreshOptions::default());
    let fleet = rig.fleet(FleetSettlementOutcome::Settled(FleetSettled {
        settled: 2,
        pruned: 3,
        joined: true,
        removed: 4,
    }));
    let removed = rig
        .children
        .settle(Some(&fleet), SessionTransition::Resume)
        .await
        .expect("settled");
    assert_eq!(removed, 4);
    assert_eq!(rig.journal(), ["fleet.settle"]);
}

#[tokio::test]
async fn unsettled_and_interrupted_runs_refuse_with_their_detail() {
    let rig = build_fresh_rig(FreshOptions::default());
    let fleet = rig.fleet(FleetSettlementOutcome::Unsettled(vec![(
        AgentUuid::new("A"),
        "still running".into(),
    )]));
    assert_eq!(
        rig.children
            .settle(Some(&fleet), SessionTransition::Fresh)
            .await
            .expect_err("unsettled"),
        SessionTransitionRefused::Unsettled(vec![(AgentUuid::new("A"), "still running".into())])
    );
    let fleet = rig.fleet(FleetSettlementOutcome::Interrupted);
    assert_eq!(
        rig.children
            .settle(Some(&fleet), SessionTransition::Fresh)
            .await
            .expect_err("interrupted"),
        SessionTransitionRefused::Interrupted
    );
}

#[tokio::test]
async fn without_a_fleet_only_a_roster_of_records_passes() {
    let rig = build_fresh_rig(FreshOptions {
        roster: Some((3, 5)),
        ..FreshOptions::default()
    });
    assert_eq!(
        rig.children
            .settle(None, SessionTransition::Fresh)
            .await
            .expect_err("live rows"),
        SessionTransitionRefused::NoFleetTeardown(3)
    );
    rig.roster.as_ref().unwrap().live.store(0, Ordering::SeqCst);
    assert_eq!(
        rig.children
            .settle(None, SessionTransition::Fresh)
            .await
            .expect("records only"),
        0
    );
    let rig = build_fresh_rig(FreshOptions {
        roster: None,
        ..FreshOptions::default()
    });
    assert_eq!(
        rig.children
            .settle(None, SessionTransition::Resume)
            .await
            .expect("no roster"),
        0
    );
}

#[test]
fn the_roster_is_replaced_only_when_no_live_delegated_row_remains() {
    let rig = build_fresh_rig(FreshOptions {
        roster: Some((1, 4)),
        ..FreshOptions::default()
    });
    assert_eq!(
        rig.children
            .reset_roster(SessionTransition::Resume)
            .expect_err("live row"),
        SessionTransitionRefused::LiveRowsRemain(1)
    );
    let roster = rig.roster.as_ref().unwrap();
    assert_eq!(roster.records.load(Ordering::SeqCst), 4, "nothing dropped");
    roster.live.store(0, Ordering::SeqCst);
    assert_eq!(
        rig.children
            .reset_roster(SessionTransition::Resume)
            .expect("records only"),
        4
    );
    assert_eq!(roster.records.load(Ordering::SeqCst), 0);
    assert_eq!(rig.journal(), ["roster.clear"]);
    let rig = build_fresh_rig(FreshOptions {
        roster: None,
        ..FreshOptions::default()
    });
    assert_eq!(
        rig.children
            .reset_roster(SessionTransition::Fresh)
            .expect("no roster"),
        0
    );
    assert!(format!("{:?}", rig.children).contains("tracks_roster: false"));
}
