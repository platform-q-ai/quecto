use super::*;

#[test]
fn terminate_helpers_ignore_zero_and_huge_pids_without_panicking() {
    terminate_owned_process_tree(0, ProcessOwner::DirectPid);
    terminate_owned_process_tree(0, ProcessOwner::LocalProcessGroup);
    terminate_owned_process_tree(u32::MAX, ProcessOwner::DirectPid);
    terminate_owned_process_tree(u32::MAX, ProcessOwner::LocalProcessGroup);
}
