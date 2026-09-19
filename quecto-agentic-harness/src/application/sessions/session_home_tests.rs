use super::*;

#[test]
fn every_obstacle_maps_onto_its_startup_disposition() {
    use ResumeDecisionKind as Kind;
    let cases = [
        (
            HomeObstacle::decision(Kind::LegacyUnscoped),
            ResumeDisposition::LegacyUnscoped,
        ),
        (
            HomeObstacle::decision(Kind::HomeChanged),
            ResumeDisposition::HomeChanged,
        ),
        (
            HomeObstacle::decision(Kind::CrossFolder),
            ResumeDisposition::DifferentExecutionDirectory,
        ),
        (
            HomeObstacle::Decision(Kind::HomeMissing, Some("gone".into())),
            ResumeDisposition::Unavailable("gone".into()),
        ),
        (
            HomeObstacle::Decision(Kind::HomeUnknown, None),
            ResumeDisposition::Unavailable(String::new()),
        ),
        (
            HomeObstacle::CurrentUnavailable("no cwd".into()),
            ResumeDisposition::Unavailable("no cwd".into()),
        ),
    ];
    for (obstacle, disposition) in cases {
        assert_eq!(
            obstacle.clone().into_disposition(),
            disposition,
            "{obstacle:?}"
        );
    }
}
