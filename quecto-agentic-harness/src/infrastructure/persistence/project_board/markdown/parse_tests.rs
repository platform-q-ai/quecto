//! The reader's own guards, judged before the file is written again: a
//! hand-edited file is refused for what it holds, whatever a re-render of
//! it would say.
use super::*;

/// The reader refuses a paragraph that would define a link reference,
/// escaped or not, on the line it opens on: the read path's guard, apart
/// from the renderer's.
#[test]
fn the_reader_refuses_a_definition_paragraph_itself() {
    for (lines, line) in [
        (vec!["\\[docs]: //evil.example/x"], 1),
        (vec!["\\[a", "b]: /x"], 1),
        (vec!["[`]: //evil.example/x \"`](https://example.com)\""], 1),
    ] {
        let mut reader = Reader {
            lines: &lines,
            at: 0,
        };
        assert_eq!(
            reader.paragraph(),
            Err(MarkdownError::ReferenceDefinition { line }),
            "{lines:?}"
        );
    }
    let lines = vec!["\\[WIP] not done: soon"];
    let mut reader = Reader {
        lines: &lines,
        at: 0,
    };
    assert_eq!(
        reader.paragraph(),
        Ok(Block::Paragraph(vec!["[WIP] not done: soon".into()]))
    );
}

/// The reader refuses a list item that opens as a definition, its label's
/// end hidden in a code span, on its own line.
#[test]
fn the_reader_refuses_a_definition_item_itself() {
    let lines = vec!["- [`]: //evil.example/x \"`](https://example.com)\""];
    let mut reader = Reader {
        lines: &lines,
        at: 0,
    };
    assert_eq!(
        reader.list(),
        Err(MarkdownError::ReferenceDefinition { line: 1 })
    );
}
