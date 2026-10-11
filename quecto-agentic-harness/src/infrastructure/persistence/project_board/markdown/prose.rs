//! Prose: paragraphs, one-level `- ` lists and fenced code, one blank line
//! between them. Nothing else a block can be survives: a paragraph's line
//! or a list item that starts as another block would is escaped with a
//! backslash before its ASCII punctuation, so it reads as text.
use super::inline::{check_inline, starts_with_link};
use super::{Block, MarkdownError};

/// The punctuation a line that starts as a block starts with: heading,
/// table, quote, setext underline, list, thematic break, definition,
/// footnote or link reference, HTML, escape, tilde fence, math. A backtick
/// needs none: a line of text closes every code span it opens, so three
/// backticks there are a span, never a fence.
pub const BLOCK_STARTS: &str = "#|>=-+*_:[<\\~$";
/// The longest language a fence names.
const MOST_LANGUAGE: usize = 20;

/// A fence's language: `a-z`, `0-9`, `+` and `-`, at most twenty.
pub fn language(name: &str) -> bool {
    (1..=MOST_LANGUAGE).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'+' || b == b'-')
}

/// The digits an ordered list's marker starts with, and the `.` or `)`
/// after them, when `line` starts as one: `1. x`, `2)`.
pub fn list_number(line: &str) -> Option<usize> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let marker = line
        .as_bytes()
        .get(digits)
        .is_some_and(|b| matches!(b, b'.' | b')'));
    let then = line.as_bytes().get(digits + 1).is_none_or(|b| *b == b' ');
    (digits > 0 && marker && then).then_some(digits)
}

/// `line` as a file holds it: a backslash before the punctuation a block
/// would start with, or before an ordered marker's `.` or `)`.
pub fn escaped(line: &str) -> String {
    match (line.chars().next(), list_number(line)) {
        // A link opens the line: a reference definition it is not.
        (Some('['), _) if starts_with_link(line) => line.to_string(),
        (Some(first), _) if BLOCK_STARTS.contains(first) => format!("\\{line}"),
        (_, Some(digits)) => format!("{}\\{}", &line[..digits], &line[digits..]),
        _ => line.to_string(),
    }
}

/// One line of text in a paragraph or list: not empty, no leading space,
/// and the inline rules.
fn text_line(text: &str, at: usize) -> Result<(), MarkdownError> {
    let expected = "a line of text with no space before it";
    match text.is_empty() || text.starts_with(' ') {
        true => Err(MarkdownError::Structure { line: at, expected }),
        false => check_inline(text, at),
    }
}

/// `blocks` appended to `lines`, one blank line between them.
///
/// # Errors
/// What in them is refused, and the file line it would land on.
pub fn render(
    lines: &mut Vec<String>,
    heading: &str,
    blocks: &[Block],
) -> Result<(), MarkdownError> {
    if blocks.is_empty() {
        return Err(MarkdownError::EmptyProse {
            heading: heading.into(),
        });
    }
    for (index, block) in blocks.iter().enumerate() {
        // Two lists a blank line apart are one list to a reader.
        let after_list = index
            .checked_sub(1)
            .is_some_and(|before| matches!(blocks[before], Block::List(_)));
        if let (true, Block::List(_)) = (after_list, block) {
            let expected = "one list, not two in a row";
            return Err(MarkdownError::Structure {
                line: lines.len() + 2,
                expected,
            });
        }
        if index > 0 {
            lines.push(String::new());
        }
        let at = lines.len() + 1;
        let empty = MarkdownError::Structure {
            line: at,
            expected: "a block that holds something",
        };
        match block {
            Block::Paragraph(texts) | Block::List(texts) if texts.is_empty() => return Err(empty),
            Block::Paragraph(texts) => {
                for text in texts {
                    text_line(text, lines.len() + 1)?;
                    lines.push(escaped(text));
                }
            }
            Block::List(items) => {
                for item in items {
                    text_line(item, lines.len() + 1)?;
                    lines.push(format!("- {}", escaped(item)));
                }
            }
            Block::Code {
                language: name,
                lines: code,
            } => {
                let fence = |why| MarkdownError::Fence { line: at, why };
                match name.as_deref().is_none_or(language) {
                    true => lines.push(format!("```{}", name.as_deref().unwrap_or(""))),
                    false => return Err(fence("a language of a-z, 0-9, + and -, at most twenty")),
                }
                for line in code {
                    match line.starts_with("```") {
                        true => {
                            return Err(MarkdownError::Fence {
                                line: lines.len() + 1,
                                why: "a line a fence would end on",
                            });
                        }
                        false => lines.push(line.clone()),
                    }
                }
                lines.push("```".into());
            }
        }
    }
    Ok(())
}
