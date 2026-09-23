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

fn render(stream: TokenStream, out: &mut String) {
    for token in stream {
        match token {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                out.push_str(open);
                render(group.stream(), out);
                out.push_str(close);
            }
            TokenTree::Ident(ident) => {
                if out.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    out.push(' ');
                }
                out.push_str(&ident.to_string());
            }
            TokenTree::Punct(punct) => out.push(punct.as_char()),
            // String, char and number literals cannot name a dependency.
            TokenTree::Literal(_) => {}
        }
    }
}

/// The file's production code as compact text (`std::fs::read(&p)`): test-only
/// items, comments and literals removed. `None` when the file does not parse,
/// which every caller treats as a violation (fail closed).
pub fn production_text(source: &str) -> Option<String> {
    let mut file = syn::parse_file(source).ok()?;
    StripTestOnly.visit_file_mut(&mut file);
    let mut out = String::new();
    render(file.into_token_stream(), &mut out);
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
