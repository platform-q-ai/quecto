use super::*;

#[test]
fn a_subsequence_is_every_character_in_order_and_nothing_matches_an_empty_term() {
    assert!(title_holds_subsequence("fxbg", "fix bug"));
    assert!(title_holds_subsequence("fix bug", "fix bug"));
    assert!(!title_holds_subsequence("fxbg", "bug fix"));
    assert!(!title_holds_subsequence("fxbgx", "fix bug"));
    assert!(!title_holds_subsequence("", "fix bug"));
    assert!(!title_holds_subsequence("f", ""));
}

#[test]
fn the_rank_is_the_fuzzy_tier_when_any_term_needed_it_else_the_best_field() {
    assert_eq!(rank_of(&[]), None);
    assert_eq!(rank_of(&[MatchedField::Title]), Some(MatchedField::Title));
    assert_eq!(
        rank_of(&[MatchedField::Key, MatchedField::TitleFuzzy]),
        Some(MatchedField::TitleFuzzy)
    );
    assert_eq!(MIN_SUBSEQUENCE_TERM_CHARS, 3);
}
