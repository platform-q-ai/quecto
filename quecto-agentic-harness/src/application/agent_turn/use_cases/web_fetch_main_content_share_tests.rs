//! #2165 review (PR #2222): a landmark's share is measured with the same
//! header treatment as the whole page's, while its own header stays in
//! what is returned.
use super::*;

fn words(n: usize, word: &str) -> String {
    format!("<p>{}</p>", vec![word; n].join(" "))
}

/// The review's case: the heading is kept, the unrelated text is not.
#[test]
fn a_heading_inside_main_does_not_make_it_too_large() {
    let text = readable_html("<main><header>HHHHHHHHHH</header><p>AAAAA</p></main><p>BBBBB</p>");
    assert!(text.starts_with(MAIN_CONTENT_NOTE), "{text}");
    assert!(
        text.contains("HHHHHHHHHH") && text.contains("AAAAA"),
        "{text}"
    );
    assert!(!text.contains("BBBBB"), "{text}");
}

/// A large header inside `<main>` does not tip it into too large.
#[test]
fn a_large_header_inside_main_does_not_tip_it_into_too_large() {
    let html = format!(
        "<html><head><title>T</title></head><body><main><header>{}</header>{}</main>{}</body></html>",
        words(200, "hd"),
        words(40, "in"),
        words(40, "sd")
    );
    let text = readable_html(&html);
    assert!(text.contains(MAIN_CONTENT_NOTE), "{text}");
    assert!(text.contains("hd hd") && text.contains("in in"), "{text}");
    assert!(!text.contains("sd"), "{text}");
}
