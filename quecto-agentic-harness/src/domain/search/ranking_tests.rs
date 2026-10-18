use super::relevance_order;

#[test]
fn most_relevant_first_then_unjudged_in_original_order() {
    assert_eq!(
        relevance_order(&[Some(0.2), None, Some(0.9), None, Some(0.5)]),
        [2, 4, 0, 1, 3]
    );
}

#[test]
fn ties_keep_their_original_order() {
    assert_eq!(
        relevance_order(&[Some(0.5), Some(0.5), Some(0.7)]),
        [2, 0, 1]
    );
    assert_eq!(relevance_order(&[None, None]), [0, 1]);
    assert!(relevance_order(&[]).is_empty());
}

#[test]
#[should_panic(expected = "relevance scores are probabilities")]
fn a_score_outside_zero_to_one_is_a_contract_breach() {
    relevance_order(&[Some(1.5)]);
}
