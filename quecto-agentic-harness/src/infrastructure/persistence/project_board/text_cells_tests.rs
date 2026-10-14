use super::*;

#[test]
fn a_text_cell_holds_plain_text_and_a_link_but_no_bare_address_or_math() {
    let cells = MarkdownTextCells;
    assert!(cells.holds("src/board.rs at 4f2a9c1"));
    assert!(cells.holds("[link](https://github.com/o/r/pull/1) at 4f2a9c1"));
    for refused in [
        "https://github.com/o/r/pull/1 at 4f2a9c1",
        "http://example.com/x at 4f2a9c1",
        "ada@example.com at 4f2a9c1",
        "costs $5 at 4f2a9c1",
        "",
    ] {
        assert!(!cells.holds(refused), "{refused:?}");
    }
}
