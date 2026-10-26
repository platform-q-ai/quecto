//! Contract for [`FollowTokens`] (epic #2705 B11 review 2) on the tokens
//! file: a server's own tokens are not a primary's set; set aside, they are
//! kept beside and refused at once; a set taken from a primary is known as
//! such when opened again, and a token issued after it makes the set the
//! server's own again.
use quecto::application::project_board::ports::BoardTokens;
use quecto::application::project_board::ports::replication::{
    AdminRows, FollowTokens, FollowedTokens,
};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::file::board_tokens::FileBoardTokens;

fn name(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

#[test]
fn own_tokens_set_aside_a_primarys_set_known_and_new_ones_own_again() {
    let dir = tempfile::tempdir().unwrap();
    let at = Timestamp::parse("2026-10-26T09:00:00Z").unwrap();
    let state = dir.path().join("follower");
    let tokens = FileBoardTokens::open(&state).unwrap();
    let held = |held, copied| FollowedTokens { held, copied };
    assert_eq!(tokens.followed(), Ok(held(0, false)));
    let laptop = tokens.add(&name("laptop"), &at).unwrap();
    assert_eq!(tokens.followed(), Ok(held(1, false)));
    tokens.set_aside().unwrap();
    assert_eq!(tokens.verify(&laptop.secret), Ok(None), "refused at once");
    assert_eq!(tokens.followed(), Ok(held(0, true)));
    assert!(state.join("tokens.json.pre-follow").exists());
    let primary = FileBoardTokens::open(&dir.path().join("primary")).unwrap();
    primary.add(&name("phone"), &at).unwrap();
    tokens.replace(&primary.export().unwrap()).unwrap();
    drop(tokens);
    let reopened = FileBoardTokens::open(&state).unwrap();
    assert_eq!(reopened.followed(), Ok(held(1, true)));
    reopened.add(&name("tablet"), &at).unwrap();
    assert_eq!(reopened.followed(), Ok(held(2, false)), "its own again");
}
