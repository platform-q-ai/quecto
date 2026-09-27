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
    assert!(text.starts_with(MAIN_CONTENT_NOTE_LEAD), "{text}");
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
    assert!(text.contains(MAIN_CONTENT_NOTE_LEAD), "{text}");
    assert!(text.contains("hd hd") && text.contains("in in"), "{text}");
    assert!(!text.contains("sd"), "{text}");
}

/// #2225: the note counts the text dropped, the whole page's text less the
/// title's and the landmark's, whitespace aside, and says how to get it.
#[test]
fn the_note_counts_the_bytes_dropped() {
    let html = "<html><head><title>T</title></head><body><main><p>in in</p></main><p>o t</p></body></html>";
    assert_eq!(
        readable_html(html),
        "T\n[Main content only; 2 bytes of page text dropped; main_only: false returns the whole page]\n\nin in"
    );
    let text = readable_html("<main><p>in in</p></main><p>ot</p>");
    assert!(
        text.starts_with(&format!("{}\n\nin in", main_content_note(2))),
        "{text}"
    );
    assert_eq!(text_bytes(" a\tb\n\nc "), 3);
}

/// #2225: past a third of the page's text a landmark is always kept, up to
/// all of it.
#[test]
fn any_substantial_share_is_kept() {
    for (inside, outside) in [(34, 66), (50, 50), (91, 9), (99, 1), (100, 0)] {
        let html = format!(
            "<main>{}</main>{}",
            words(inside, "in"),
            words(outside, "ot")
        );
        let text = readable_html(&html);
        assert!(text.starts_with(MAIN_CONTENT_NOTE_LEAD), "{inside}: {text}");
        assert!(!text.contains("ot"), "{inside}: {text}");
        assert!(
            text.contains(&main_content_note(2 * outside)),
            "{inside}: {text}"
        );
    }
}

#[test]
fn the_note_is_one_line_naming_the_whole_page_option() {
    for dropped in [0, 7, usize::MAX] {
        let note = main_content_note(dropped);
        assert!(note.starts_with(MAIN_CONTENT_NOTE_LEAD), "{note}");
        assert!(note.contains(&format!("{dropped} bytes")), "{note}");
        assert!(
            note.contains("main_only: false") && !note.contains('\n'),
            "{note}"
        );
    }
}

/// A title with no text gives no title line: the note leads.
#[test]
fn an_empty_title_gives_no_line() {
    let html = format!(
        "<html><head><title> </title></head><body><main>{}</main>{}</body></html>",
        words(60, "in"),
        words(20, "ot")
    );
    let text = readable_html(&html);
    assert!(text.starts_with(MAIN_CONTENT_NOTE_LEAD), "{text:?}");
    assert!(text.contains(&main_content_note(40)), "{text}");
}
