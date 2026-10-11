//! Prose: paragraphs, one-level `- ` lists and fenced code, one blank line
//! between them. Nothing else a block can be survives: a paragraph's line
//! or a list item that opens with ASCII punctuation (the backtick aside),
//! or as an ordered marker, is escaped with a backslash, so it reads as
//! text; an item opens with `[` only as a link, never a task box.
use super::chars::check_line;
use super::inline::{check_inline, starts_with_link};
use super::{Block, MarkdownError};

/// Whether a line opening with `first` is escaped: every ASCII punctuation
/// (a backslash escape is valid before each), so no block any reader knows
/// can start, but the backtick: a line of text closes every code span it
/// opens, so three backticks there are a span, never a fence, and an
/// escape would leave its span unopened.
pub fn opens_escaped(first: char) -> bool {
    first.is_ascii_punctuation() && first != '`'
}
/// The languages a fence may name: plain highlighters, each of which a
/// reader (GitHub's included) shows as code. Others (`math`, `mermaid`,
/// `geojson`, `topojson`, `stl`) are drawn as something else.
pub const LANGUAGES: &[&str] = &[
    "text", "sh", "bash", "console", "rust", "toml", "json", "yaml", "diff", "python", "js", "ts",
    "kotlin", "java", "go", "sql", "html", "css", "markdown", "c", "cpp",
];

/// Why a fence's language is refused, as render and parse both say it.
pub const LANGUAGE_RULE: &str = "a language from the list of plain highlighters";

/// A fence's language: one of [`LANGUAGES`].
pub fn language(name: &str) -> bool {
    LANGUAGES.contains(&name)
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
        (Some(first), _) if opens_escaped(first) => format!("\\{line}"),
        (_, Some(digits)) => format!("{}\\{}", &line[..digits], &line[digits..]),
        _ => line.to_string(),
    }
}

/// The text a line of a paragraph or list item holds: [`escaped`] undone.
/// `None` when the line starts as a block would, unescaped.
pub fn unescaped(line: &str) -> Option<String> {
    let mut chars = line.chars();
    let (first, second) = (chars.next(), chars.next());
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    // An ordered marker the writer escaped: digits, `\`, then `.` or `)`.
    let marker = (digits > 0 && line[digits..].starts_with('\\'))
        .then(|| format!("{}{}", &line[..digits], &line[digits + 1..]))
        .filter(|marker| list_number(marker) == Some(digits));
    match (first, second, marker) {
        (Some('\\'), Some(next), _) if opens_escaped(next) => Some(line[1..].to_string()),
        (_, _, Some(marker)) => Some(marker),
        (Some('['), ..) if starts_with_link(line) => Some(line.to_string()),
        (Some(first), ..) if opens_escaped(first) => None,
        _ if list_number(line).is_some() => None,
        _ => Some(line.to_string()),
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
                    // A `[` opening an item is a task box to GitHub, but
                    // for a link's.
                    if item.starts_with('[') && !starts_with_link(item) {
                        return Err(MarkdownError::Structure {
                            line: lines.len() + 1,
                            expected: "an item that opens with `[` only as a link",
                        });
                    }
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
                    false => return Err(fence(LANGUAGE_RULE)),
                }
                for line in code {
                    // A reader closes the fence on a line of three or more
                    // backticks after up to three spaces; any line that
                    // starts with three, after any spaces, is refused.
                    match line.trim_start_matches(' ').starts_with("```") {
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

/// A domain's markdown text as blocks: chunks one blank line apart, each
/// fenced code (its blank lines kept), a list when every line of it starts
/// `- `, else a paragraph of its lines verbatim. The empty text is no
/// blocks (its section left out). [`text_of`] gives the text back.
///
/// # Errors
/// Text the blocks cannot hold as it is: a blank line at an edge or two in
/// a row (a final newline among them), a fence left open or a line straight
/// after one, and what the blocks' own rules (two lists in a row, a line of
/// prose a space starts, a language off the list) and the character rules
/// (a tab, a no-break space) refuse. A line is the text's, from 1.
pub fn blocks_of(text: &str) -> Result<Vec<Block>, MarkdownError> {
    if text.is_empty() {
        return Ok(vec![]);
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let apart = |at: usize| MarkdownError::Structure {
        line: at + 1,
        expected: "blocks one blank line apart, none at an edge",
    };
    let (mut blocks, mut at) = (vec![], 0);
    loop {
        let first = match lines.get(at) {
            Some(first) if !first.is_empty() => *first,
            _ => return Err(apart(at)),
        };
        // A fence as a reader opens one: its info holds no backtick.
        let (block, next) = match first.strip_prefix("```").filter(|info| !info.contains('`')) {
            Some(info) => code_of(&lines, at, info)?,
            None => {
                let end = lines[at..]
                    .iter()
                    .position(|line| line.is_empty())
                    .map_or(lines.len(), |length| at + length);
                let chunk = &lines[at..end];
                let items: Option<Vec<String>> = chunk
                    .iter()
                    .map(|line| line.strip_prefix("- ").map(str::to_string))
                    .collect();
                let block = match items {
                    Some(items) => Block::List(items),
                    None => Block::Paragraph(chunk.iter().map(|line| line.to_string()).collect()),
                };
                (block, end)
            }
        };
        blocks.push(block);
        match (lines.get(next), lines.get(next + 1)) {
            (None, _) => break,
            (Some(&""), Some(after)) if !after.is_empty() => at = next + 1,
            _ => return Err(apart(next)),
        }
    }
    // The blocks' own rules and the characters', on the very lines a file
    // would hold (one for each of the text's).
    let mut rendered = vec![];
    render(&mut rendered, "text", &blocks)?;
    debug_assert_eq!(
        rendered.len(),
        lines.len(),
        "a line of the file per line of text"
    );
    for (index, line) in rendered.iter().enumerate() {
        check_line(line, index + 1)?;
    }
    // Every chunk is kept whole (a paragraph's lines verbatim, a list's
    // items after their `- `, a fence's lines and info), one blank line
    // apart, so writing the blocks back gives the text.
    debug_assert_eq!(text_of(&blocks), text, "blocks give their text back");
    Ok(blocks)
}

/// The fenced code `lines[at]` opens with `info`, and the line after its
/// closing fence.
fn code_of(lines: &[&str], at: usize, info: &str) -> Result<(Block, usize), MarkdownError> {
    let inner = &lines[at + 1..];
    let close = inner
        .iter()
        .position(|line| *line == "```")
        .ok_or(MarkdownError::Fence {
            line: at + 1,
            why: "a fence left open",
        })?;
    let language = match info {
        "" => None,
        name => Some(name.to_string()),
    };
    let code = inner[..close].iter().map(|line| line.to_string()).collect();
    let block = Block::Code {
        language,
        lines: code,
    };
    Ok((block, at + close + 2))
}

/// The text `blocks` hold, one blank line between them: what
/// [`blocks_of`] reads them from.
pub fn text_of(blocks: &[Block]) -> String {
    let block = |block: &Block| match block {
        Block::Paragraph(lines) => lines.join("\n"),
        Block::List(items) => items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Code { language, lines } => {
            let open = format!("```{}", language.as_deref().unwrap_or(""));
            std::iter::once(open)
                .chain(lines.iter().cloned())
                .chain(std::iter::once("```".to_string()))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    blocks.iter().map(block).collect::<Vec<_>>().join("\n\n")
}
