//! The oracle's second reader: comrak, a port of cmark-gfm, reading a
//! file as GitHub does, outlined block by block as the first reader's
//! [`super::Outline`] is.
use super::*;
use comrak::nodes::{AstNode, ListType, NodeValue};

/// comrak's options: what GitHub reads: autolinks, tables,
/// strikethrough, task lists, footnotes, `$` and code math, alerts, and
/// the tag filter.
fn comrak_options() -> comrak::Options<'static> {
    let mut options = comrak::Options::default();
    let extension = &mut options.extension;
    extension.autolink = true;
    extension.table = true;
    extension.strikethrough = true;
    extension.tasklist = true;
    extension.footnotes = true;
    extension.math_dollars = true;
    extension.math_code = true;
    extension.alerts = true;
    extension.tagfilter = true;
    options
}

/// The inline text `node` shows, and where each link in it starts and
/// ends in that text (each link's text added to `link_texts`). Only text,
/// code, emphasis, strikethrough, soft breaks (never in a link) and inline
/// https links with no title are allowed.
fn comrak_inline<'a>(
    node: &'a AstNode<'a>,
    link_texts: &mut Vec<String>,
    in_link: bool,
) -> Result<(String, Vec<std::ops::Range<usize>>), String> {
    let (mut text, mut links) = (String::new(), vec![]);
    for child in node.children() {
        match &child.data.borrow().value {
            NodeValue::Text(shown) => text.push_str(shown),
            NodeValue::Code(code) => text.push_str(&spanned(&code.literal)),
            NodeValue::SoftBreak if !in_link => text.push('\n'),
            NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough => {
                let (shown, inner) = comrak_inline(child, link_texts, in_link)?;
                let at = text.len();
                links.extend(
                    inner
                        .into_iter()
                        .map(|range| range.start + at..range.end + at),
                );
                text.push_str(&shown);
            }
            NodeValue::Link(link)
                if link.url.starts_with("https://") && link.title.is_empty() && !in_link =>
            {
                let (shown, _) = comrak_inline(child, &mut vec![], true)?;
                link_texts.push(shown.clone());
                links.push(text.len()..text.len() + shown.len());
                text.push_str(&shown);
            }
            other => return Err(format!("{other:?}")),
        }
    }
    Ok((text, links))
}

/// comrak's outline of `file`, block by block, as [`Outline`] keeps it.
pub(super) fn comrak_outline(file: &str) -> Result<Read, String> {
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, file, &comrak_options());
    let (mut kinds, mut texts, mut link_texts, mut link_runs) = (vec![], vec![], vec![], vec![]);
    // The inline text of `node`, and how many links it holds, the runs of
    // text around them kept.
    let mut inline = |node, link_texts: &mut Vec<String>| -> Result<(String, usize), String> {
        let (text, ranges) = comrak_inline(node, link_texts, false)?;
        link_runs.extend(runs(&text, &ranges));
        Ok((text, ranges.len()))
    };
    for block in root.children() {
        let (kind, text, links) = match &block.data.borrow().value {
            NodeValue::HtmlBlock(_) if kinds.is_empty() => ("marker".to_string(), String::new(), 0),
            NodeValue::Heading(heading) if heading.level == 1 && kinds.len() == 1 => {
                let (text, links) = inline(block, &mut link_texts)?;
                ("h1".to_string(), text, links)
            }
            NodeValue::Heading(heading) if heading.level == 2 => {
                let (text, links) = inline(block, &mut link_texts)?;
                ("h2".to_string(), text, links)
            }
            NodeValue::Paragraph => {
                let (text, links) = inline(block, &mut link_texts)?;
                (
                    format!("paragraph:{}", text.matches('\n').count() + 1),
                    text,
                    links,
                )
            }
            NodeValue::List(list) if list.list_type == ListType::Bullet => {
                let (mut text, mut links, mut items) = (String::new(), 0, 0);
                for item in block.children() {
                    let paragraphs: Vec<_> = item.children().collect();
                    match (&item.data.borrow().value, paragraphs.as_slice()) {
                        (NodeValue::Item(_), [paragraph])
                            if matches!(paragraph.data.borrow().value, NodeValue::Paragraph) =>
                        {
                            let (shown, inner) = inline(paragraph, &mut link_texts)?;
                            text += &format!("{shown}\n");
                            links += inner;
                            items += 1;
                        }
                        (other, _) => return Err(format!("{other:?} in a list")),
                    }
                }
                (format!("list:{items}"), text, links)
            }
            NodeValue::CodeBlock(code) if code.fenced => (
                format!("code:{}", code.literal.matches('\n').count()),
                code.literal.clone(),
                0,
            ),
            NodeValue::Table(_) => comrak_table(block, &mut link_texts, &mut inline)?,
            other => return Err(format!("{other:?}")),
        };
        kinds.push(format!("{kind}+{links}"));
        texts.push(text);
    }
    Ok((kinds, texts, link_texts, link_runs))
}

/// A table's kind, text and links, as [`Outline`] keeps them.
fn comrak_table<'a>(
    table: &'a AstNode<'a>,
    link_texts: &mut Vec<String>,
    inline: &mut impl FnMut(&'a AstNode<'a>, &mut Vec<String>) -> Result<(String, usize), String>,
) -> Result<(String, String, usize), String> {
    let (mut text, mut links, mut rows, mut width, mut ragged) = (String::new(), 0, 0, None, false);
    for row in table.children() {
        let header = match &row.data.borrow().value {
            NodeValue::TableRow(header) => *header,
            other => return Err(format!("{other:?} in a table")),
        };
        let mut cells = vec![];
        for cell in row.children() {
            let (shown, inner) = inline(cell, link_texts)?;
            cells.push(shown);
            links += inner;
        }
        match width {
            None => width = Some(cells.len()),
            Some(header) => ragged |= header != cells.len(),
        }
        rows += usize::from(!header);
        text += &(cells.join("\t") + "\n");
    }
    let kind = match ragged {
        true => "table:ragged".to_string(),
        false => format!("table:{}x{rows}", width.unwrap_or(0)),
    };
    Ok((kind, text, links))
}
