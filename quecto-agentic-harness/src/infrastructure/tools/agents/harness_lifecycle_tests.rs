use super::*;

#[test]
fn only_an_accepting_harness_admits_a_spawn() {
    let cell = new_shared_harness_lifecycle();
    assert_eq!(admit_spawn(&cell), Ok(()));
    *cell.lock().unwrap() = HarnessLifecycleState::Frozen;
    assert_eq!(
        admit_spawn(&cell),
        Err(SpawnRefused {
            state: HarnessLifecycleState::Frozen
        })
    );
    *cell.lock().unwrap() = HarnessLifecycleState::Terminated;
    let refused = admit_spawn(&cell).unwrap_err();
    assert_eq!(refused.state, HarnessLifecycleState::Terminated);
    assert!(refused.to_string().contains("Terminated"));
    assert_eq!(current(&cell), HarnessLifecycleState::Terminated);
}
