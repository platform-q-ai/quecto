//! Test-only seams of the idle drain (#562 nudge injection, guarded turn
//! admission): a test installs a one-shot hook the loop runs at that point.
static BEFORE_WORKFLOW_NUDGE_INJECTION_TEST_HOOK: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> =
    std::sync::Mutex::new(None);
static BEFORE_GUARDED_TURN_ADMISSION_TEST_HOOK: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> =
    std::sync::Mutex::new(None);

pub(super) fn run_before_workflow_nudge_injection_test_hook() {
    if let Some(hook) = BEFORE_WORKFLOW_NUDGE_INJECTION_TEST_HOOK
        .lock()
        .unwrap()
        .take()
    {
        hook();
    }
}

pub(in crate::interface::cli) fn set_before_workflow_nudge_injection_test_hook(
    hook: Box<dyn Fn() + Send>,
) {
    *BEFORE_WORKFLOW_NUDGE_INJECTION_TEST_HOOK.lock().unwrap() = Some(hook);
}

pub(super) fn run_before_guarded_turn_admission_test_hook() {
    if let Some(hook) = BEFORE_GUARDED_TURN_ADMISSION_TEST_HOOK
        .lock()
        .unwrap()
        .take()
    {
        hook();
    }
}

pub(in crate::interface::cli) fn set_before_guarded_turn_admission_test_hook(
    hook: Box<dyn Fn() + Send>,
) {
    *BEFORE_GUARDED_TURN_ADMISSION_TEST_HOOK.lock().unwrap() = Some(hook);
}
