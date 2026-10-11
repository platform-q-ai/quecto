//! Text as the board keeps it: CRLF line ends turned to LF once, here, for
//! every front end (#2477), before the task's schema reads it (the schema
//! refuses `\r`).
use crate::domain::project_board::entities::task_parts::PlanStep;

pub(super) fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

pub(super) fn plan_lf(plan: &[PlanStep]) -> Vec<PlanStep> {
    plan.iter()
        .map(|step| PlanStep {
            title: lf(&step.title),
            criteria: step.criteria.iter().map(|text| lf(text)).collect(),
        })
        .collect()
}
