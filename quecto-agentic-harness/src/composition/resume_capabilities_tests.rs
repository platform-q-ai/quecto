use crate::application::sessions::dto::ActionAvailability;
use crate::domain::resume_decision::ResumeAction;

#[test]
fn only_cancel_is_executable_until_an_executor_is_composed() {
    let composed = super::composed();
    for action in ResumeAction::ALL {
        let available = composed.availability(action) == ActionAvailability::Available;
        assert_eq!(available, action == ResumeAction::Cancel, "{action:?}");
    }
}
