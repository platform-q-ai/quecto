use super::*;

/// Pairs that share a branch or pull request without contradiction pass
/// every rule: a merged change and an open one on one branch, and an
/// unlinked change and the change that links its pull request again.
#[test]
fn changes_that_share_a_branch_or_pull_request_in_turn_agree() {
    let ada = person("Ada");
    let mut again = change(2, None).into_fields();
    again.branch = branch("feat/b1");
    let after_merge = [recorded(1, None, 'a'), Change::new(again).unwrap()];
    let none = HostPrs::new();
    assert!(
        link(
            &of(InProgress),
            on(&after_merge),
            head("feat/z", None),
            &none,
            &ada,
            &now()
        )
        .is_ok()
    );
    assert!(unlink(&of(InProgress), &after_merge, 2, &none, &ada, &now()).is_ok());
    let mut relinked = change(2, Some(7)).into_fields();
    relinked.branch = branch("feat/b2");
    let same_pr = [unlinked(1, 7), Change::new(relinked).unwrap()];
    let open = host(&[(7, "feat/b2", PrState::Open)]);
    assert!(
        link(
            &of(InProgress),
            on(&same_pr),
            head("feat/z", None),
            &open,
            &ada,
            &now()
        )
        .is_ok()
    );
    assert!(unlink(&of(InProgress), &same_pr, 2, &open, &ada, &now()).is_ok());
    let merged_now = host(&[(7, "feat/b2", merged('b'))]);
    assert!(complete(&of(Review), &same_pr, &merged_now, &[], &now()).is_ok());
}
