use super::*;

#[test]
fn only_get_and_post_are_admitted() {
    assert_eq!(admit("GET", &[], false).unwrap().method, Method::Get);
    assert_eq!(admit("POST", &[], false).unwrap().method, Method::Post);
    for method in ["get", "PUT", "DELETE", "HEAD", "OPTIONS", "BREW", ""] {
        assert_eq!(admit(method, &[], false), Err(Refusal::Method), "{method}");
    }
}

#[test]
fn a_body_is_admitted_only_by_one_plain_length_up_to_one_mebibyte() {
    let most = MAX_BODY_BYTES.to_string();
    assert_eq!(
        admit("POST", &[most.as_bytes()], false).unwrap().body_bytes,
        MAX_BODY_BYTES
    );
    let over = (MAX_BODY_BYTES + 1).to_string();
    assert_eq!(
        admit("POST", &[over.as_bytes()], false),
        Err(Refusal::TooLarge)
    );
    for bad in [
        &b""[..],
        b"-1",
        b"+1",
        b"1 ",
        b"0x10",
        b"99999999999999999999",
    ] {
        assert_eq!(admit("POST", &[bad], false), Err(Refusal::BadLength));
    }
    assert_eq!(admit("POST", &[b"1", b"1"], false), Err(Refusal::BadLength));
    assert_eq!(admit("POST", &[b"1"], true), Err(Refusal::LengthRequired));
}

#[test]
fn each_refusal_has_its_status() {
    let statuses = [
        Refusal::Method,
        Refusal::LengthRequired,
        Refusal::BadLength,
        Refusal::TooLarge,
    ]
    .map(Refusal::status);
    assert_eq!(statuses, [405, 411, 400, 413]);
}
