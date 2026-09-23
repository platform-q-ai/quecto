//! Production tokens of a Rust source file (#1637), shared by the
//! architecture rules, the BDD architecture steps and the catalogue
//! conformance checks: every `#[cfg(test)]` item is removed wherever it sits
//! (not only at the end of the file), comments are gone and string literals
//! are dropped, so neither a test module in the middle of a file nor a comment
//! or string can hide or fake a dependency. Its fixtures live in
//! `tests/architecture/dependency_scan.rs`.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit_mut::VisitMut;

/// `#[cfg(test)]` exactly — the one attribute the layer rules treat as
/// test-only. Anything else (including `cfg(any(test, …))`) is production.
pub fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

pub fn item_test_only(item: &syn::Item) -> bool {
    let attrs = match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::ForeignMod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::TraitAlias(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        // Verbatim tokens carry no parsed attributes: production.
        _ => return false,
    };
    test_only(attrs)
}

struct StripTestOnly;

impl VisitMut for StripTestOnly {
    fn visit_file_mut(&mut self, file: &mut syn::File) {
        file.items.retain(|item| !item_test_only(item));
        syn::visit_mut::visit_file_mut(self, file);
    }
    fn visit_item_mod_mut(&mut self, module: &mut syn::ItemMod) {
        if let Some((_, items)) = &mut module.content {
            items.retain(|item| !item_test_only(item));
        }
        syn::visit_mut::visit_item_mod_mut(self, module);
    }
    fn visit_item_impl_mut(&mut self, block: &mut syn::ItemImpl) {
        block.items.retain(|item| match item {
            syn::ImplItem::Const(i) => !test_only(&i.attrs),
            syn::ImplItem::Fn(i) => !test_only(&i.attrs),
            syn::ImplItem::Type(i) => !test_only(&i.attrs),
            syn::ImplItem::Macro(i) => !test_only(&i.attrs),
            _ => true,
        });
        syn::visit_mut::visit_item_impl_mut(self, block);
    }
    fn visit_block_mut(&mut self, block: &mut syn::Block) {
        block.stmts.retain(|stmt| match stmt {
            syn::Stmt::Item(item) => !item_test_only(item),
            syn::Stmt::Local(local) => !test_only(&local.attrs),
            _ => true,
        });
        syn::visit_mut::visit_block_mut(self, block);
    }
}

/// Attribute keys whose string value is Rust code naming paths
/// (`#[serde(with = "crate::codec")]`, `bound = "T: crate::Tr"`). Every
/// other attribute string (`#[error("…")]`, `reason = "…"`, doc text) is
/// prose and never matches.
pub const PATH_KEYS: [&str; 12] = [
    "with",
    "serialize_with",
    "deserialize_with",
    "default",
    "from",
    "into",
    "try_from",
    "remote",
    "crate",
    "bound",
    "getter",
    "path",
];

/// True when `tokens[at]` is a literal written as `key = "…"` for one of
/// [`PATH_KEYS`].
pub fn is_path_valued(tokens: &[TokenTree], at: usize) -> bool {
    at >= 2
        && matches!(&tokens[at - 1], TokenTree::Punct(eq) if eq.as_char() == '=')
        && matches!(&tokens[at - 2],
            TokenTree::Ident(key) if PATH_KEYS.contains(&key.to_string().as_str()))
}

/// `attribute`: inside `#[…]`, where a path-valued string (see
/// [`PATH_KEYS`]) is rendered as the code it holds; every other literal is
/// dropped, as it cannot name a dependency.
fn render(stream: TokenStream, out: &mut String, attribute: bool) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut previous_hash = false;
    for (at, token) in tokens.iter().enumerate() {
        let hash = matches!(token, TokenTree::Punct(p) if p.as_char() == '#')
            || (previous_hash && matches!(token, TokenTree::Punct(p) if p.as_char() == '!'));
        match token {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                let opens_attribute = previous_hash && group.delimiter() == Delimiter::Bracket;
                out.push_str(open);
                render(group.stream(), out, attribute || opens_attribute);
                out.push_str(close);
            }
            TokenTree::Ident(ident) => {
                if out.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    out.push(' ');
                }
                out.push_str(&ident.to_string());
            }
            TokenTree::Punct(punct) => out.push(punct.as_char()),
            TokenTree::Literal(literal) if attribute && is_path_valued(&tokens, at) => {
                if let Ok(text) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                    out.extend(text.value().chars().filter(|c| !c.is_whitespace()));
                }
            }
            TokenTree::Literal(_) => {}
        }
        previous_hash = hash;
    }
}

/// The file's production code as compact text (`std::fs::read(&p)`): test-only
/// items, comments and literals (outside non-doc attributes) removed. `None` when the file does not parse,
/// which every caller treats as a violation (fail closed).
pub fn production_text(source: &str) -> Option<String> {
    let mut file = syn::parse_file(source).ok()?;
    StripTestOnly.visit_file_mut(&mut file);
    let mut out = String::new();
    render(file.into_token_stream(), &mut out, false);
    Some(out)
}

/// The first forbidden pattern in the file's production code, with context.
pub fn forbidden_hit(source: &str, forbidden: &[&str]) -> Result<(), String> {
    let text = production_text(source).ok_or_else(|| "file does not parse".to_string())?;
    for pattern in forbidden {
        if let Some(at) = text.find(pattern) {
            let before: String = text[..at].chars().rev().take(60).collect();
            let after: String = text[at..].chars().take(pattern.len() + 60).collect();
            let before: String = before.chars().rev().collect();
            return Err(format!("forbidden pattern {pattern} in: …{before}{after}…"));
        }
    }
    Ok(())
}

/// Panicking form for the per-file rules.
pub fn assert_no_forbidden(what: &str, source: &str, forbidden: &[&str]) {
    if let Err(hit) = forbidden_hit(source, forbidden) {
        panic!("{what}: {hit}");
    }
}

/// A source position: 1-based line, 0-based column in chars.
pub type Position = (usize, usize);

/// The spans (start, end-exclusive) of every `#[cfg(test)]` item (with its
/// attributes) wherever it sits — file, inline module, impl or block — for
/// scans that report line numbers. `None` when the file does not parse.
pub fn test_only_spans(source: &str) -> Option<Vec<(Position, Position)>> {
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    struct Ranges(Vec<(Position, Position)>);
    impl Ranges {
        fn add(&mut self, node: &impl Spanned) {
            let span = node.span();
            let (start, end) = (span.start(), span.end());
            self.0
                .push(((start.line, start.column), (end.line, end.column)));
        }
    }
    impl<'ast> Visit<'ast> for Ranges {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            if item_test_only(item) {
                self.add(item);
                return;
            }
            syn::visit::visit_item(self, item);
        }
        fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
            let attrs = match item {
                syn::ImplItem::Const(i) => &i.attrs,
                syn::ImplItem::Fn(i) => &i.attrs,
                syn::ImplItem::Type(i) => &i.attrs,
                syn::ImplItem::Macro(i) => &i.attrs,
                _ => {
                    syn::visit::visit_impl_item(self, item);
                    return;
                }
            };
            if test_only(attrs) {
                self.add(item);
                return;
            }
            syn::visit::visit_impl_item(self, item);
        }
        fn visit_local(&mut self, local: &'ast syn::Local) {
            if test_only(&local.attrs) {
                self.add(local);
                return;
            }
            syn::visit::visit_local(self, local);
        }
    }
    let file = syn::parse_file(source).ok()?;
    let mut ranges = Ranges(Vec::new());
    ranges.visit_file(&file);
    Some(ranges.0)
}

/// `source`'s lines with every `#[cfg(test)]` item blanked by column (a
/// production call sharing a line with a test item survives), keeping the
/// original 1-based line numbers; lines left blank are dropped. `None` when
/// the file does not parse.
pub fn production_lines(source: &str) -> Option<Vec<(usize, String)>> {
    let spans = test_only_spans(source)?;
    let hidden = |line: usize, column: usize| {
        spans
            .iter()
            .any(|&(start, end)| (line, column) >= start && (line, column) < end)
    };
    Some(
        source
            .lines()
            .enumerate()
            .map(|(index, text)| {
                let line = index + 1;
                let kept: String = text
                    .chars()
                    .enumerate()
                    .map(|(column, c)| if hidden(line, column) { ' ' } else { c })
                    .collect();
                (line, kept.trim_end().to_string())
            })
            .filter(|(_, text)| !text.trim().is_empty())
            .collect(),
    )
}
