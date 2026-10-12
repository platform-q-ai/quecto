//! What a value, a type or a binding holds, for the spawn check's reader:
//! a spawner, positively none, or not known ([`State`]).

use std::collections::BTreeSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::ext::IdentExt;
use syn::visit::Visit;

/// The types that spawn: a runtime, its handle, a join set, a thread
/// builder or scope, a thread pool, a task tracker.
const SPAWNER_TYPES: &[&str] = &[
    "Handle",
    "Runtime",
    "JoinSet",
    "Builder",
    "ThreadPool",
    "Scope",
    "TaskTracker",
];

/// The names a spawner goes by: a value whose words hold one of these is
/// taken for a spawner unless it is a local known to hold none.
const SPAWNER_NAMES: &[&str] = &["handle", "runtime", "builder", "thread"];

/// What a local, a parameter or a value holds. Ordered: an assignment
/// only ever raises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum State {
    /// Positively no spawner: a literal, a concrete type that names none,
    /// or a constructor of such a type.
    KnownNot,
    /// Not known: read by its name, as the merged check read every value.
    Unknown,
    /// A spawner.
    Spawner,
}

pub(super) fn is_type(word: &str) -> bool {
    SPAWNER_TYPES.contains(&word)
}

pub(super) fn is_name(word: &str) -> bool {
    SPAWNER_NAMES.contains(&word)
}

/// An identifier as written, `r#` taken off.
pub(super) fn word(ident: &proc_macro2::Ident) -> String {
    ident.unraw().to_string()
}

/// The words of `tokens`, groups flattened, raw identifiers unrawed.
pub(super) fn token_words(tokens: TokenStream) -> Vec<String> {
    let mut words = vec![];
    for tree in tokens {
        match tree {
            TokenTree::Group(group) => words.extend(token_words(group.stream())),
            TokenTree::Ident(ident) => words.push(word(&ident)),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
    words
}

/// Whether a type names a spawner.
pub(super) fn names_spawner_type(ty: &syn::Type) -> bool {
    token_words(ty.to_token_stream())
        .iter()
        .any(|word| is_type(word))
}

/// What a value of type `ty` holds: a spawner when it names one, none
/// when it is concrete (no `impl`, `dyn`, `Self`, `_` or generic parameter
/// in it), else not known.
pub(super) fn type_state(ty: &syn::Type, generic: &dyn Fn(&str) -> bool) -> State {
    let words = token_words(ty.to_token_stream());
    let open = matches!(ty, syn::Type::Infer(_))
        || words
            .iter()
            .any(|word| matches!(word.as_str(), "impl" | "dyn" | "Self" | "_") || generic(word));
    match (names_spawner_type(ty), open) {
        (true, _) => State::Spawner,
        (false, true) => State::Unknown,
        (false, false) => State::KnownNot,
    }
}

/// Whether `word` names a concrete type that is no spawner: a type name
/// (capitalised), not a spawner's, `Self` or a generic parameter.
fn plain_type(word: &str, generic: &dyn Fn(&str) -> bool) -> bool {
    word.starts_with(|c: char| c.is_ascii_uppercase())
        && !is_type(word)
        && word != "Self"
        && !generic(word)
}

/// Whether `expr` positively holds no spawner: a literal, a local known to
/// hold none, a constructor (`new`, `new_*`, `with_*`, `from`) of a plain
/// type (`Command::new("x")`) whose arguments hold none, or a cast to a
/// concrete plain type.
pub(super) fn known_not(
    expr: &syn::Expr,
    local: &dyn Fn(&str) -> Option<State>,
    generic: &dyn Fn(&str) -> bool,
) -> bool {
    let all = |mut exprs: syn::punctuated::Iter<'_, syn::Expr>| {
        exprs.all(|expr| known_not(expr, local, generic))
    };
    match expr {
        syn::Expr::Lit(_) => true,
        syn::Expr::Path(path) => match (&path.qself, path.path.segments.len()) {
            (None, 1) => local(&word(&path.path.segments[0].ident)) == Some(State::KnownNot),
            (_, _) => false,
        },
        syn::Expr::Call(call) => match &*call.func {
            syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() >= 2 => {
                let segments = &path.path.segments;
                let owner = word(&segments[segments.len() - 2].ident);
                let made = word(&segments[segments.len() - 1].ident);
                // A constructor, not a trait's (`Default::default()` is any
                // type's).
                let constructs = made == "new"
                    || made == "from"
                    || made.starts_with("new_")
                    || made.starts_with("with_");
                constructs && plain_type(&owner, generic) && all(call.args.iter())
            }
            _ => false,
        },
        syn::Expr::Reference(inner) => known_not(&inner.expr, local, generic),
        syn::Expr::Paren(inner) => known_not(&inner.expr, local, generic),
        syn::Expr::Group(inner) => known_not(&inner.expr, local, generic),
        syn::Expr::Try(inner) => known_not(&inner.expr, local, generic),
        syn::Expr::Await(inner) => known_not(&inner.base, local, generic),
        syn::Expr::Unary(inner) => known_not(&inner.expr, local, generic),
        syn::Expr::Cast(cast) => type_state(&cast.ty, generic) == State::KnownNot,
        syn::Expr::Array(array) => all(array.elems.iter()),
        syn::Expr::Tuple(tuple) => all(tuple.elems.iter()),
        // Anything else may hold a spawner somewhere: not known. A method
        // on a value known to hold none may still answer one
        // (`ctx.worker()`).
        syn::Expr::MethodCall(_)
        | syn::Expr::Assign(_)
        | syn::Expr::Async(_)
        | syn::Expr::Binary(_)
        | syn::Expr::Block(_)
        | syn::Expr::Break(_)
        | syn::Expr::Closure(_)
        | syn::Expr::Const(_)
        | syn::Expr::Continue(_)
        | syn::Expr::Field(_)
        | syn::Expr::ForLoop(_)
        | syn::Expr::If(_)
        | syn::Expr::Index(_)
        | syn::Expr::Infer(_)
        | syn::Expr::Let(_)
        | syn::Expr::Loop(_)
        | syn::Expr::Macro(_)
        | syn::Expr::Match(_)
        | syn::Expr::Range(_)
        | syn::Expr::RawAddr(_)
        | syn::Expr::Repeat(_)
        | syn::Expr::Return(_)
        | syn::Expr::Struct(_)
        | syn::Expr::TryBlock(_)
        | syn::Expr::Unsafe(_)
        | syn::Expr::Verbatim(_)
        | syn::Expr::While(_)
        | syn::Expr::Yield(_) => false,
        // syn's `Expr` is non-exhaustive: a form it adds later is not known.
        _ => false,
    }
}

/// The names a pattern binds, each with what it holds: `state` for the
/// whole value, or the binding's own declared type when that tells
/// (`h: Handle`, `c: Command`), or a spawner for a struct field named for
/// one (`S { handle, .. }`).
pub(super) fn bound_names(
    pat: &syn::Pat,
    state: State,
    type_state: &dyn Fn(&syn::Type) -> State,
    out: &mut Vec<(String, State)>,
) {
    let mut each = |pats: &mut dyn Iterator<Item = &syn::Pat>| {
        for pat in pats {
            bound_names(pat, state, type_state, out);
        }
    };
    match pat {
        syn::Pat::Ident(ident) => {
            out.push((word(&ident.ident), state));
            // `h @ Some(_)`: the subpattern's names hold what it does.
            if let Some((_, subpat)) = &ident.subpat {
                bound_names(subpat, state, type_state, out);
            }
        }
        syn::Pat::Type(typed) => {
            let declared = match type_state(&typed.ty) {
                State::Unknown => state,
                told @ (State::Spawner | State::KnownNot) => told,
            };
            bound_names(&typed.pat, declared, type_state, out);
        }
        syn::Pat::Reference(reference) => bound_names(&reference.pat, state, type_state, out),
        syn::Pat::Paren(paren) => bound_names(&paren.pat, state, type_state, out),
        syn::Pat::Tuple(tuple) => each(&mut tuple.elems.iter()),
        syn::Pat::TupleStruct(tuple) => each(&mut tuple.elems.iter()),
        syn::Pat::Slice(slice) => each(&mut slice.elems.iter()),
        syn::Pat::Or(or) => each(&mut or.cases.iter()),
        syn::Pat::Struct(fields) => {
            for field in &fields.fields {
                let named = match &field.member {
                    syn::Member::Named(name) => {
                        let name = word(name);
                        is_name(&name) || is_type(&name)
                    }
                    syn::Member::Unnamed(_) => false,
                };
                let held = match named {
                    true => State::Spawner,
                    false => state,
                };
                bound_names(&field.pat, held, type_state, out);
            }
        }
        // Patterns that bind no name.
        syn::Pat::Const(_)
        | syn::Pat::Lit(_)
        | syn::Pat::Macro(_)
        | syn::Pat::Path(_)
        | syn::Pat::Range(_)
        | syn::Pat::Rest(_)
        | syn::Pat::Verbatim(_)
        | syn::Pat::Wild(_) => {}
        // syn's `Pat` is non-exhaustive: a form it adds later binds none
        // this reader knows of.
        _ => {}
    }
}

/// The first segments of an expression's paths (`h` in `h.clone()`, `self`
/// in `self.handle`): the only words looked up as locals.
#[derive(Default)]
pub(super) struct Bases(pub(super) BTreeSet<String>);

impl<'ast> Visit<'ast> for Bases {
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if let (None, Some(first)) = (&path.qself, path.path.segments.first()) {
            self.0.insert(word(&first.ident));
        }
        syn::visit::visit_expr_path(self, path);
    }
}
