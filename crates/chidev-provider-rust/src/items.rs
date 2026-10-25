//! The Rust provider of Items. It speaks `chidev-items 1`: one JSON request `{protocol, source}` in, one JSON reply
//! `{protocol, items: [{key, lines}]}` out, and nothing else. Like the provider of mutants it is a pure function of
//! the request: it touches no file, no process and no clock.
//!
//! An Item is a declaration of the source, found by its **key** and the **lines** it is declared on. The lines
//! point at the declaration alone: from its first token (after the doc comments and attributes) to the line that
//! ends the signature, with the `;` or the `{` of its body. Two declarations on one line have lines that overlap,
//! and that is a right answer.
//!
//! # What is found
//!
//! A `fn`, `struct`, `enum`, `union`, `trait`, `type`, `const` and `static` item, a `macro_rules!` (its key is
//! `name!`), and the `fn`, `const` and `type` inside an `impl` or a `trait`. A declaration with no body
//! (`fn f() -> u64;`, which an Interface holds when it is the code a Case is written against) is not an item to
//! the Rust parser, so it is parsed again as a declaration. Nothing else is found: not a `mod` itself (an inline
//! one is entered), not an `impl` block, not what is inside a function body, not a `use`.
//!
//! # The key
//!
//! The key is a name and a path, never the text of the declaration:
//!
//! | where | key |
//! |---|---|
//! | free, or in a `mod m` | `name`, `m::name` |
//! | in an inherent `impl` | `Type<Args>::name` |
//! | in a `trait Trait` | `Trait::name` |
//! | in an `impl Trait<Args> for Type<Args>` | `<Type<Args> as Trait<Args>>::name` |
//!
//! The arguments of the type are in the key, so `impl Foo<u8>` and `impl Foo<u16>` give different keys, and so do
//! the names of the generics of the `impl` that appear there (`impl<T> Foo<T>` is `Foo<T>::f`). A type that is not
//! a path is put in angle brackets (`<[u8]>::f`). Inside a `mod` every key above is led by the path of the `mod`
//! (`m::Type::f`, `m::<Type as Trait>::f`).
//!
//! # How a key is written
//!
//! A key is an identity, not a display, so it is not rustdoc's text and does not lean on a pretty-printer. It is
//! printed from the syntax tree of the source with two rules and no exception:
//!
//! 1. A list of elements (generic arguments, the types of a tuple, the arguments of a function type) has its `,`
//!    between the elements only: a trailing `,` is dropped, so a formatter that adds or removes it, or breaks the
//!    list over lines, changes no key. The one exception to "only between" is the tuple of one element, `(u8,)`,
//!    which keeps its `,` because without it the type is `(u8)`, another type.
//! 2. The tokens are put one after another with **one space between two words and none anywhere else**. A word is
//!    an identifier, a keyword, a lifetime or a literal.
//!
//! So `<Box<dyn Error> as From<&str>>::from` is `<Box<dyn Error>as From<&str>>::from`, `&'a mut T` stays as it is,
//! and `fn() -> u8` is `fn()->u8`. A path is printed as it was written (not shortened, not made absolute), and a
//! lifetime is kept. White space in the source, and where it breaks a line, is not read. Two declarations that are
//! told apart only by what the key leaves out (a `struct Foo` and a `fn Foo`, which live in different namespaces,
//! or two `fn f` under different `#[cfg]`) get one key, and chidev refuses that as `interface.key-taken`.
//!
//! The keys of the mutants of [`crate`] are made by another function and are not these: those are kept in the
//! records of a measurement, and changing them would cut the record from its history.

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use syn::ext::IdentExt;
use syn::parse::discouraged::Speculative;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use thiserror::Error;

/// The protocol version this provider speaks.
pub const PROTOCOL: u32 = 1;

/// What chidev asks for: the Items of `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The protocol version; this provider answers only [`PROTOCOL`].
    pub protocol: u32,
    /// The code of the Interface.
    pub source: String,
}

/// One declaration found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// The name the Item is matched by when the code is put again.
    pub key: String,
    /// The first and last line of the declaration, from 1, both inside the source.
    pub lines: [usize; 2],
}

/// What the provider answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// The protocol version.
    pub protocol: u32,
    /// The declarations, in the order of the source: a `trait` or a `mod`'s item before what is inside it.
    pub items: Vec<Item>,
}

/// Why no reply could be made.
#[derive(Debug, Error)]
pub enum Error {
    /// The request names a protocol version this provider does not speak.
    #[error("protocol {0} is not spoken here; this provider speaks {PROTOCOL}")]
    Protocol(u32),
    /// The request is not a request.
    #[error("not a chidev-items request: {0}")]
    Request(serde_json::Error),
    /// The source is not Rust that parses.
    #[error("the source does not parse as Rust: {0}")]
    Source(String),
    /// The reply cannot be written.
    #[error("cannot write the reply: {0}")]
    Reply(serde_json::Error),
}

/// Reads one request from `input` (JSON) and writes the reply (JSON): the whole of the protocol, for a binary to
/// connect to its standard input and output.
pub fn handle(input: &[u8]) -> Result<Vec<u8>, Error> {
    let request: Request = serde_json::from_slice(input).map_err(Error::Request)?;
    let reply = items(&request)?;
    serde_json::to_vec(&reply).map_err(Error::Reply)
}

/// The Items of the request's source.
pub fn items(request: &Request) -> Result<Reply, Error> {
    if request.protocol != PROTOCOL {
        return Err(Error::Protocol(request.protocol));
    }
    let file = syn::parse_file(&request.source).map_err(|e| Error::Source(e.to_string()))?;
    let mut walk = Walk::default();
    walk.items(&file.items);
    Ok(Reply {
        protocol: PROTOCOL,
        items: walk.found,
    })
}

/// Walks the declarations of a file and keeps the Items.
#[derive(Default)]
struct Walk {
    /// The `mod`s the walk is inside, outermost first.
    path: Vec<String>,
    found: Vec<Item>,
}

impl Walk {
    fn items(&mut self, items: &[syn::Item]) {
        for item in items {
            self.item(item);
        }
    }

    /// Keeps the Item `name` of the scope `owner` (a type, a trait; none for a free item) when its tokens run from
    /// their first one (past the attributes) to `end`.
    fn keep(&mut self, owner: Option<&str>, name: &str, tokens: TokenStream, end: Span) {
        let Some(first) = first_line(tokens) else {
            return;
        };
        let key = match owner {
            Some(owner) => format!("{owner}::{name}"),
            None => self.scoped(name),
        };
        self.found.push(Item {
            key,
            lines: [first, end.end().line.max(first)],
        });
    }

    /// `name` led by the path of the `mod`s the walk is in.
    fn scoped(&self, name: &str) -> String {
        self.path.iter().fold(String::new(), |mut key, m| {
            key.push_str(m);
            key.push_str("::");
            key
        }) + name
    }

    fn item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Fn(f) => self.keep(
                None,
                &f.sig.ident.to_string(),
                f.to_token_stream(),
                f.block.brace_token.span.open(),
            ),
            syn::Item::Struct(s) => {
                let end = match &s.fields {
                    syn::Fields::Named(named) => named.brace_token.span.open(),
                    _ => semi_or(s.semi_token.as_ref(), s.ident.span()),
                };
                self.keep(None, &s.ident.to_string(), s.to_token_stream(), end);
            }
            syn::Item::Enum(e) => self.keep(
                None,
                &e.ident.to_string(),
                e.to_token_stream(),
                e.brace_token.span.open(),
            ),
            syn::Item::Union(u) => self.keep(
                None,
                &u.ident.to_string(),
                u.to_token_stream(),
                u.fields.brace_token.span.open(),
            ),
            syn::Item::Type(t) => self.keep(
                None,
                &t.ident.to_string(),
                t.to_token_stream(),
                t.semi_token.span(),
            ),
            syn::Item::Const(c) => self.keep(
                None,
                &c.ident.to_string(),
                c.to_token_stream(),
                c.semi_token.span(),
            ),
            syn::Item::Static(s) => self.keep(
                None,
                &s.ident.to_string(),
                s.to_token_stream(),
                s.semi_token.span(),
            ),
            syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => {
                if let Some(name) = &m.ident {
                    self.keep(
                        None,
                        &format!("{name}!"),
                        m.to_token_stream(),
                        m.mac.delimiter.span().open(),
                    );
                }
            }
            syn::Item::Trait(t) => {
                self.keep(
                    None,
                    &t.ident.to_string(),
                    t.to_token_stream(),
                    t.brace_token.span.open(),
                );
                let owner = self.scoped(&t.ident.to_string());
                for member in &t.items {
                    self.trait_item(&owner, member);
                }
            }
            syn::Item::Impl(i) => {
                let owner = self.impl_owner(i);
                for member in &i.items {
                    self.impl_item(&owner, member);
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    self.path.push(m.ident.to_string());
                    self.items(items);
                    self.path.pop();
                }
            }
            syn::Item::Verbatim(tokens) => self.declared(None, tokens.clone()),
            _ => {}
        }
    }

    fn trait_item(&mut self, owner: &str, member: &syn::TraitItem) {
        match member {
            syn::TraitItem::Fn(f) => {
                let end = match &f.default {
                    Some(block) => block.brace_token.span.open(),
                    None => semi_or(f.semi_token.as_ref(), f.sig.ident.span()),
                };
                self.keep(
                    Some(owner),
                    &f.sig.ident.to_string(),
                    f.to_token_stream(),
                    end,
                );
            }
            syn::TraitItem::Const(c) => self.keep(
                Some(owner),
                &c.ident.to_string(),
                c.to_token_stream(),
                c.semi_token.span(),
            ),
            syn::TraitItem::Type(t) => self.keep(
                Some(owner),
                &t.ident.to_string(),
                t.to_token_stream(),
                t.semi_token.span(),
            ),
            syn::TraitItem::Verbatim(tokens) => self.declared(Some(owner), tokens.clone()),
            _ => {}
        }
    }

    fn impl_item(&mut self, owner: &str, member: &syn::ImplItem) {
        match member {
            syn::ImplItem::Fn(f) => self.keep(
                Some(owner),
                &f.sig.ident.to_string(),
                f.to_token_stream(),
                f.block.brace_token.span.open(),
            ),
            syn::ImplItem::Const(c) => self.keep(
                Some(owner),
                &c.ident.to_string(),
                c.to_token_stream(),
                c.semi_token.span(),
            ),
            syn::ImplItem::Type(t) => self.keep(
                Some(owner),
                &t.ident.to_string(),
                t.to_token_stream(),
                t.semi_token.span(),
            ),
            syn::ImplItem::Verbatim(tokens) => self.declared(Some(owner), tokens.clone()),
            _ => {}
        }
    }

    /// A declaration with no body, which the parser leaves as tokens (`pub fn f() -> u64;`, `const N: usize;`):
    /// parsed again as the declaration it is. Tokens that are not one are not an Item.
    fn declared(&mut self, owner: Option<&str>, tokens: TokenStream) {
        if let Ok(d) = syn::parse2::<Bodyless>(tokens) {
            let key = match owner {
                Some(owner) => format!("{owner}::{}", d.name),
                None => self.scoped(&d.name),
            };
            self.found.push(Item {
                key,
                lines: [d.first, d.end.max(d.first)],
            });
        }
    }

    /// What leads the key of a member of `i`: the type for an inherent `impl`, `<Type as Trait>` for a trait's,
    /// both led by the `mod`s the walk is in.
    fn impl_owner(&self, i: &syn::ItemImpl) -> String {
        let mut p = Printer::default();
        match &i.trait_ {
            Some((_, path, _)) => {
                p.punct("<");
                p.ty(&i.self_ty);
                p.word("as");
                p.path(path);
                p.punct(">");
            }
            None => match &*i.self_ty {
                syn::Type::Path(t) if t.qself.is_none() => p.ty(&i.self_ty),
                other => {
                    p.punct("<");
                    p.ty(other);
                    p.punct(">");
                }
            },
        }
        self.scoped(&p.finish())
    }
}

/// The span of a `;`, or `fallback` when there is none.
fn semi_or(semi: Option<&syn::Token![;]>, fallback: Span) -> Span {
    semi.map_or(fallback, |s| s.span())
}

/// The line of the first token of `tokens` that is not an attribute (a doc comment is one).
fn first_line(tokens: TokenStream) -> Option<usize> {
    let mut trees = tokens.into_iter().peekable();
    while let Some(tree) = trees.peek() {
        let TokenTree::Punct(p) = tree else {
            return Some(tree.span().start().line);
        };
        if p.as_char() != '#' {
            return Some(p.span().start().line);
        }
        trees.next();
        if matches!(trees.peek(), Some(TokenTree::Punct(bang)) if bang.as_char() == '!') {
            trees.next();
        }
        match trees.next() {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => {}
            other => return other.map(|t| t.span().start().line),
        }
    }
    None
}

/// A declaration that has no body, as it is parsed again: its name, the line of its first token past the
/// attributes, and the line of its `;`.
struct Bodyless {
    name: String,
    first: usize,
    end: usize,
}

impl Parse for Bodyless {
    fn parse(input: ParseStream) -> syn::Result<Bodyless> {
        input.call(syn::Attribute::parse_outer)?;
        let first = input.cursor().span().start().line;
        input.parse::<syn::Visibility>()?;
        input.parse::<Option<syn::Token![default]>>()?;
        // `const fn f();` is a function and `const N: usize;` is not: a signature is tried first
        let ahead = input.fork();
        let name = if let Ok(sig) = ahead.parse::<syn::Signature>() {
            input.advance_to(&ahead);
            sig.ident.to_string()
        } else if input.peek(syn::Token![static]) {
            input.parse::<syn::Token![static]>()?;
            input.parse::<Option<syn::Token![mut]>>()?;
            input.parse::<syn::Ident>()?.to_string()
        } else if input.peek(syn::Token![const]) {
            input.parse::<syn::Token![const]>()?;
            input.call(syn::Ident::parse_any)?.to_string()
        } else if input.peek(syn::Token![type]) {
            input.parse::<syn::Token![type]>()?;
            input.parse::<syn::Ident>()?.to_string()
        } else {
            return Err(input.error("not a declaration with no body"));
        };
        // the rest of the signature: up to the `;`
        while !input.peek(syn::Token![;]) {
            input.parse::<TokenTree>()?;
        }
        let end = input.parse::<syn::Token![;]>()?.span().end().line;
        Ok(Bodyless { name, first, end })
    }
}

/// Writes a key. Two rules, no exception: a list has its `,` between its elements only (a tuple of one element
/// also has the one after it), and a space stands between two words and nowhere else.
#[derive(Default)]
struct Printer {
    out: String,
    /// Whether the last token written was a word.
    word_last: bool,
}

impl Printer {
    fn finish(self) -> String {
        self.out
    }

    fn word(&mut self, word: &str) {
        if self.word_last {
            self.out.push(' ');
        }
        self.out.push_str(word);
        self.word_last = true;
    }

    fn punct(&mut self, punct: &str) {
        self.out.push_str(punct);
        self.word_last = false;
    }

    fn lifetime(&mut self, lifetime: &syn::Lifetime) {
        self.word(&format!("'{}", lifetime.ident));
    }

    /// The tokens of something the printer has no rule for (an expression in a const argument, a macro type), one
    /// after another by rule 2. A `,` in it is written as it is.
    fn tokens(&mut self, stream: TokenStream) {
        let mut trees = stream.into_iter().peekable();
        while let Some(tree) = trees.next() {
            match tree {
                TokenTree::Ident(i) => self.word(&i.to_string()),
                TokenTree::Literal(l) => self.word(&l.to_string()),
                TokenTree::Punct(p) if p.as_char() == '\'' => match trees.peek() {
                    Some(TokenTree::Ident(i)) => {
                        let lifetime = format!("'{i}");
                        trees.next();
                        self.word(&lifetime);
                    }
                    _ => self.punct("'"),
                },
                TokenTree::Punct(p) => self.punct(&p.as_char().to_string()),
                TokenTree::Group(g) => {
                    let (open, close) = match g.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::None => ("", ""),
                    };
                    self.punct(open);
                    self.tokens(g.stream());
                    self.punct(close);
                }
            }
        }
    }

    /// `each` of `elements`, with a `,` between.
    fn list<T>(
        &mut self,
        elements: impl IntoIterator<Item = T>,
        mut each: impl FnMut(&mut Self, T),
    ) {
        for (n, element) in elements.into_iter().enumerate() {
            if n > 0 {
                self.punct(",");
            }
            each(self, element);
        }
    }

    fn ty(&mut self, ty: &syn::Type) {
        match ty {
            syn::Type::Array(a) => {
                self.punct("[");
                self.ty(&a.elem);
                self.punct(";");
                self.tokens(a.len.to_token_stream());
                self.punct("]");
            }
            syn::Type::BareFn(f) => {
                if let Some(lifetimes) = &f.lifetimes {
                    self.bound_lifetimes(lifetimes);
                }
                if f.unsafety.is_some() {
                    self.word("unsafe");
                }
                if let Some(abi) = &f.abi {
                    self.word("extern");
                    if let Some(name) = &abi.name {
                        self.word(&name.token().to_string());
                    }
                }
                self.word("fn");
                self.punct("(");
                self.list(&f.inputs, |p, arg| {
                    if let Some((name, _)) = &arg.name {
                        p.word(&name.to_string());
                        p.punct(":");
                    }
                    p.ty(&arg.ty);
                });
                if let Some(variadic) = &f.variadic {
                    if !f.inputs.is_empty() {
                        self.punct(",");
                    }
                    if let Some((name, _)) = &variadic.name {
                        self.word(&name.to_string());
                        self.punct(":");
                    }
                    self.punct("...");
                }
                self.punct(")");
                self.output(&f.output);
            }
            syn::Type::Group(g) => self.ty(&g.elem),
            syn::Type::ImplTrait(t) => {
                self.word("impl");
                self.bounds(&t.bounds);
            }
            syn::Type::Infer(_) => self.punct("_"),
            syn::Type::Never(_) => self.punct("!"),
            syn::Type::Paren(p) => {
                self.punct("(");
                self.ty(&p.elem);
                self.punct(")");
            }
            syn::Type::Path(p) => self.type_path(p),
            syn::Type::Ptr(p) => {
                self.punct("*");
                self.word(if p.mutability.is_some() {
                    "mut"
                } else {
                    "const"
                });
                self.ty(&p.elem);
            }
            syn::Type::Reference(r) => {
                self.punct("&");
                if let Some(lifetime) = &r.lifetime {
                    self.lifetime(lifetime);
                }
                if r.mutability.is_some() {
                    self.word("mut");
                }
                self.ty(&r.elem);
            }
            syn::Type::Slice(s) => {
                self.punct("[");
                self.ty(&s.elem);
                self.punct("]");
            }
            syn::Type::TraitObject(t) => {
                if t.dyn_token.is_some() {
                    self.word("dyn");
                }
                self.bounds(&t.bounds);
            }
            syn::Type::Tuple(t) => {
                self.punct("(");
                self.list(&t.elems, |p, e| p.ty(e));
                if t.elems.len() == 1 {
                    self.punct(",");
                }
                self.punct(")");
            }
            other => self.tokens(other.to_token_stream()),
        }
    }

    fn output(&mut self, output: &syn::ReturnType) {
        if let syn::ReturnType::Type(_, ty) = output {
            self.punct("->");
            self.ty(ty);
        }
    }

    fn bound_lifetimes(&mut self, bound: &syn::BoundLifetimes) {
        self.word("for");
        self.punct("<");
        self.list(&bound.lifetimes, |p, param| match param {
            syn::GenericParam::Lifetime(l) => {
                p.lifetime(&l.lifetime);
                if !l.bounds.is_empty() {
                    p.punct(":");
                    p.list(&l.bounds, |p, b| p.lifetime(b));
                }
            }
            other => p.tokens(other.to_token_stream()),
        });
        self.punct(">");
    }

    /// Bounds joined by `+`.
    fn bounds(
        &mut self,
        bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    ) {
        for (n, bound) in bounds.iter().enumerate() {
            if n > 0 {
                self.punct("+");
            }
            match bound {
                syn::TypeParamBound::Trait(t) => {
                    if t.paren_token.is_some() {
                        self.punct("(");
                    }
                    if matches!(t.modifier, syn::TraitBoundModifier::Maybe(_)) {
                        self.punct("?");
                    }
                    if let Some(lifetimes) = &t.lifetimes {
                        self.bound_lifetimes(lifetimes);
                    }
                    self.path(&t.path);
                    if t.paren_token.is_some() {
                        self.punct(")");
                    }
                }
                syn::TypeParamBound::Lifetime(l) => self.lifetime(l),
                other => self.tokens(other.to_token_stream()),
            }
        }
    }

    fn type_path(&mut self, p: &syn::TypePath) {
        let Some(q) = &p.qself else {
            return self.path(&p.path);
        };
        // `<T as Trait>::Rest`: the first `position` segments are the trait
        self.punct("<");
        self.ty(&q.ty);
        if q.position > 0 {
            self.word("as");
            self.segments(
                p.path.leading_colon.is_some(),
                p.path.segments.iter().take(q.position),
            );
        }
        self.punct(">");
        for segment in p.path.segments.iter().skip(q.position) {
            self.punct("::");
            self.segment(segment);
        }
    }

    fn path(&mut self, path: &syn::Path) {
        self.segments(path.leading_colon.is_some(), path.segments.iter());
    }

    fn segments<'a>(
        &mut self,
        leading: bool,
        segments: impl Iterator<Item = &'a syn::PathSegment>,
    ) {
        if leading {
            self.punct("::");
        }
        for (n, segment) in segments.enumerate() {
            if n > 0 {
                self.punct("::");
            }
            self.segment(segment);
        }
    }

    fn segment(&mut self, segment: &syn::PathSegment) {
        self.word(&segment.ident.to_string());
        match &segment.arguments {
            syn::PathArguments::None => {}
            syn::PathArguments::AngleBracketed(a) => self.angle(a),
            syn::PathArguments::Parenthesized(a) => {
                self.punct("(");
                self.list(&a.inputs, |p, t| p.ty(t));
                self.punct(")");
                self.output(&a.output);
            }
        }
    }

    fn angle(&mut self, a: &syn::AngleBracketedGenericArguments) {
        if a.colon2_token.is_some() {
            self.punct("::");
        }
        self.punct("<");
        self.list(&a.args, |p, arg| match arg {
            syn::GenericArgument::Lifetime(l) => p.lifetime(l),
            syn::GenericArgument::Type(t) => p.ty(t),
            syn::GenericArgument::Const(e) => p.tokens(e.to_token_stream()),
            syn::GenericArgument::AssocType(b) => {
                p.word(&b.ident.to_string());
                if let Some(generics) = &b.generics {
                    p.angle(generics);
                }
                p.punct("=");
                p.ty(&b.ty);
            }
            syn::GenericArgument::AssocConst(b) => {
                p.word(&b.ident.to_string());
                if let Some(generics) = &b.generics {
                    p.angle(generics);
                }
                p.punct("=");
                p.tokens(b.value.to_token_stream());
            }
            syn::GenericArgument::Constraint(c) => {
                p.word(&c.ident.to_string());
                if let Some(generics) = &c.generics {
                    p.angle(generics);
                }
                p.punct(":");
                p.bounds(&c.bounds);
            }
            other => p.tokens(other.to_token_stream()),
        });
        self.punct(">");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(source: &str) -> Vec<Item> {
        items(&Request {
            protocol: PROTOCOL,
            source: source.to_string(),
        })
        .unwrap_or_else(|e| panic!("no reply for {source:?}: {e}"))
        .items
    }

    fn keys(source: &str) -> Vec<String> {
        ask(source).into_iter().map(|i| i.key).collect()
    }

    fn found(source: &str) -> Vec<(String, [usize; 2])> {
        ask(source).into_iter().map(|i| (i.key, i.lines)).collect()
    }

    /// The key of the one `fn f` in an `impl` with the header `header`.
    fn key_of_impl(header: &str) -> String {
        let source = format!("{header} {{ fn f() {{}} }}");
        let all = keys(&source);
        assert_eq!(all.len(), 1, "{header}: {all:?}");
        all.into_iter().next().unwrap()
    }

    #[test]
    fn a_free_fn_or_type_is_its_name_and_a_mod_leads_it_with_its_path() {
        let source = "fn a() {}\nstruct B;\nmod m {\n    fn c() {}\n    mod n {\n        enum D {}\n    }\n}\nconst E: u8 = 1;\n";
        assert_eq!(keys(source), ["a", "B", "m::c", "m::n::D", "E"]);
    }

    #[test]
    fn every_kind_of_item_is_found_with_the_lines_of_its_declaration() {
        let source = "\
/// A function.
#[inline]
pub fn price(
    total: u64,
) -> u64 {
    total
}

struct Unit;
struct Tuple(
    u8,
);
pub struct Named {
    n: u8,
}
enum Kind {
    A,
}
union Bits {
    a: u8,
}
trait Shape {
    fn area(&self) -> u64;
}
type Alias = u8;
const LIMIT: u8 = 1;
static COUNT: u8 = 1;
macro_rules! twice {
    ($e:expr) => { $e * 2 };
}
";
        assert_eq!(
            found(source),
            [
                ("price".to_string(), [3, 5]),
                ("Unit".to_string(), [9, 9]),
                ("Tuple".to_string(), [10, 12]),
                ("Named".to_string(), [13, 13]),
                ("Kind".to_string(), [16, 16]),
                ("Bits".to_string(), [19, 19]),
                ("Shape".to_string(), [22, 22]),
                ("Shape::area".to_string(), [23, 23]),
                ("Alias".to_string(), [25, 25]),
                ("LIMIT".to_string(), [26, 26]),
                ("COUNT".to_string(), [27, 27]),
                ("twice!".to_string(), [28, 28]),
            ]
        );
    }

    #[test]
    fn the_lines_start_after_the_doc_comments_and_attributes() {
        let source = "/// One.\n/// Two.\n#[must_use]\n#[cfg(unix)]\nfn f() -> u8 {\n    1\n}\n";
        assert_eq!(found(source), [("f".to_string(), [5, 5])]);
    }

    #[test]
    fn the_lines_end_at_the_line_of_the_semicolon_or_the_brace_of_the_body() {
        let source =
            "fn f<T>(x: T)\nwhere\n    T: Copy,\n{\n    let _ = x;\n}\ntype Long =\n    u8;\n";
        assert_eq!(
            found(source),
            [("f".to_string(), [1, 4]), ("Long".to_string(), [7, 8])]
        );
    }

    #[test]
    fn members_of_an_impl_and_a_trait_are_found_whatever_their_kind() {
        let source = "\
trait Tr<T> {
    type Out;
    const N: usize;
    fn make() -> T;
    fn name(&self) -> u8 { 1 }
}
struct S;
impl S {
    const K: u8 = 1;
    fn new() -> S { S }
}
impl Tr<u8> for S {
    type Out = u8;
    const N: usize = 1;
    fn make() -> u8 { 1 }
}
";
        assert_eq!(
            keys(source),
            [
                "Tr",
                "Tr::Out",
                "Tr::N",
                "Tr::make",
                "Tr::name",
                "S",
                "S::K",
                "S::new",
                "<S as Tr<u8>>::Out",
                "<S as Tr<u8>>::N",
                "<S as Tr<u8>>::make",
            ]
        );
    }

    #[test]
    fn a_mod_leads_the_key_of_what_is_in_an_impl_or_a_trait_in_it() {
        let source = "mod m {\n    trait Tr { fn a(); }\n    struct S;\n    impl S { fn b() {} }\n    impl Tr for S { fn a() {} }\n}\n";
        assert_eq!(
            keys(source),
            ["m::Tr", "m::Tr::a", "m::S", "m::S::b", "m::<S as Tr>::a"]
        );
    }

    #[test]
    fn what_a_key_does_not_name_is_not_found() {
        // a `mod` itself, an `impl` block, a `use`, a macro call, and what a function body holds
        let source = "mod outer;\nmod inner {}\nuse std::fmt;\nthread_local! { static X: u8 = 1; }\nfn f() {\n    fn nested() {}\n    struct Local;\n}\nimpl Drop for S { fn drop(&mut self) {} }\nstruct S;\nextern \"C\" {\n    fn c();\n}\n";
        assert_eq!(keys(source), ["f", "<S as Drop>::drop", "S"]);
    }

    #[test]
    fn the_inherent_impls_of_one_type_with_other_arguments_are_other_keys() {
        let source = "impl Foo<u8> { fn f() {} }\nimpl Foo<u16> { fn f() {} }\nimpl<T> Foo<T> { fn f() {} }\n";
        assert_eq!(keys(source), ["Foo<u8>::f", "Foo<u16>::f", "Foo<T>::f"]);
    }

    #[test]
    fn an_impl_for_a_type_that_is_not_a_path_puts_the_type_in_angle_brackets() {
        assert_eq!(key_of_impl("impl [u8]"), "<[u8]>::f");
        assert_eq!(key_of_impl("impl<T> *const T"), "<*const T>::f");
        assert_eq!(key_of_impl("impl dyn Shape"), "<dyn Shape>::f");
        assert_eq!(key_of_impl("impl (u8, u16)"), "<(u8,u16)>::f");
    }

    #[test]
    fn a_declaration_with_no_body_is_parsed_again_as_a_declaration() {
        let source = "\
/// The price.
pub fn price(total: u64) -> u64;
const fn square(n: u64) -> u64;
unsafe extern \"C\" fn raw(
    p: *const u8,
) -> u8;
const LIMIT: u64;
static COUNT: u64;
type Money;
";
        assert_eq!(
            found(source),
            [
                ("price".to_string(), [2, 2]),
                ("square".to_string(), [3, 3]),
                ("raw".to_string(), [4, 6]),
                ("LIMIT".to_string(), [7, 7]),
                ("COUNT".to_string(), [8, 8]),
                ("Money".to_string(), [9, 9]),
            ]
        );
    }

    #[test]
    fn a_bodyless_declaration_inside_a_mod_and_an_impl_is_found_too() {
        let source = "mod m {\n    pub fn f() -> u8;\n}\nimpl S {\n    pub fn g() -> u8;\n}\n";
        assert_eq!(keys(source), ["m::f", "S::g"]);
    }

    #[test]
    fn two_declarations_on_one_line_have_lines_that_overlap() {
        let source = "struct A; struct B;\npub trait Clock { fn now(&self) -> u64; }\n";
        assert_eq!(
            found(source),
            [
                ("A".to_string(), [1, 1]),
                ("B".to_string(), [1, 1]),
                ("Clock".to_string(), [2, 2]),
                ("Clock::now".to_string(), [2, 2]),
            ]
        );
    }

    #[test]
    fn declarations_of_the_two_namespaces_get_one_key_and_the_domain_refuses_it() {
        // the provider reports both; `interface.key-taken` is chidev's to raise
        assert_eq!(keys("struct Foo;\nfn Foo() {}\n"), ["Foo", "Foo"]);
        assert_eq!(
            keys("#[cfg(unix)]\nfn f() {}\n#[cfg(windows)]\nfn f() {}\n"),
            ["f", "f"]
        );
    }

    #[test]
    fn the_examples_of_the_design_are_their_keys() {
        for (header, key) in [
            (
                "impl From<&str> for Box<dyn Error>",
                "<Box<dyn Error>as From<&str>>::f",
            ),
            ("impl AsRef<[u8]> for [u8]", "<[u8]as AsRef<[u8]>>::f"),
            ("impl<'a, T> Foo<&'a mut T>", "Foo<&'a mut T>::f"),
            ("impl Foo<[u8; 4]>", "Foo<[u8;4]>::f"),
            ("impl<'a> Foo<'a>", "Foo<'a>::f"),
            ("impl Foo<for<'a> fn(&'a u8)>", "Foo<for<'a>fn(&'a u8)>::f"),
            ("impl<T> Foo<&mut T>", "Foo<&mut T>::f"),
            ("impl Foo<&dyn Error>", "Foo<&dyn Error>::f"),
            ("impl<T> Foo<*mut T>", "Foo<*mut T>::f"),
            (
                "impl<E> From<Box<dyn Error + Send + Sync>> for E",
                "<E as From<Box<dyn Error+Send+Sync>>>::f",
            ),
            ("impl Foo<fn() -> u8>", "Foo<fn()->u8>::f"),
            (
                "impl Foo<Box<dyn for<'a> Fn(&'a u8)>>",
                "Foo<Box<dyn for<'a>Fn(&'a u8)>>::f",
            ),
            (
                "impl<I> Iterator<Item = u8> for I",
                "<I as Iterator<Item=u8>>::f",
            ),
            ("impl<'a> Foo<&'a [u8]>", "Foo<&'a[u8]>::f"),
            ("impl<'a, 'b> Foo<&'a &'b u8>", "Foo<&'a&'b u8>::f"),
            (
                "impl Foo<unsafe extern \"C\" fn()>",
                "Foo<unsafe extern \"C\" fn()>::f",
            ),
        ] {
            assert_eq!(key_of_impl(header), key, "{header}");
        }
    }

    #[test]
    fn the_white_space_of_the_source_changes_no_key() {
        let tight =
            "impl<'a,T:Clone>From<&'a[T]>for Foo<&'a mut[T;4],Box<dyn Error+Send>>{fn f(){}}";
        let airy = "impl < 'a , T : Clone >\n    From < & 'a [ T ] >\n    for Foo <\n        & 'a mut [ T ; 4 ] ,\n        Box < dyn Error + Send > ,\n    >\n{\n    fn f ( ) { }\n}\n";
        assert_eq!(keys(tight), keys(airy));
        assert_eq!(
            keys(airy),
            ["<Foo<&'a mut[T;4],Box<dyn Error+Send>>as From<&'a[T]>>::f"]
        );
    }

    #[test]
    fn a_trailing_comma_over_several_lines_is_the_same_key_as_none() {
        let broken = "impl<A, B> Foo<\n    A,\n    B,\n> {\n    fn f() {}\n}\n";
        let plain = "impl<A, B> Foo<A, B> { fn f() {} }";
        assert_eq!(keys(broken), ["Foo<A,B>::f"]);
        assert_eq!(keys(broken), keys(plain));
        let tupled = "impl Foo<(\n    A,\n    B,\n)> { fn f() {} }";
        assert_eq!(keys(tupled), ["Foo<(A,B)>::f"]);
        let function = "impl Foo<fn(\n    A,\n    B,\n) -> C> { fn f() {} }";
        assert_eq!(keys(function), ["Foo<fn(A,B)->C>::f"]);
    }

    #[test]
    fn a_tuple_of_one_element_keeps_its_comma_and_is_not_the_parenthesised_type() {
        assert_eq!(key_of_impl("impl Foo<(u8,)>"), "Foo<(u8,)>::f");
        assert_eq!(key_of_impl("impl Foo<(u8)>"), "Foo<(u8)>::f");
        assert_ne!(
            key_of_impl("impl Foo<(u8,)>"),
            key_of_impl("impl Foo<(u8)>")
        );
        // written over lines, with or without the trailing comma, the tuple of one is the same key
        assert_eq!(key_of_impl("impl Foo<(\n    u8\n,)>"), "Foo<(u8,)>::f");
        assert_eq!(key_of_impl("impl Foo<()>"), "Foo<()>::f");
    }

    #[test]
    fn a_path_is_printed_as_it_was_written() {
        assert_eq!(
            key_of_impl("impl std::fmt::Display for crate::money::Money"),
            "<crate::money::Money as std::fmt::Display>::f"
        );
        assert_eq!(
            key_of_impl("impl ::core::fmt::Debug for Foo"),
            "<Foo as::core::fmt::Debug>::f"
        );
        assert_eq!(
            key_of_impl("impl<T> Foo<<T as Tr>::Out>"),
            "Foo<<T as Tr>::Out>::f"
        );
        // the arguments of `Fn(..)` are not a tuple: a trailing `,` is dropped
        assert_eq!(
            key_of_impl("impl Foo<Box<dyn Fn(u8,) -> u8>>"),
            "Foo<Box<dyn Fn(u8)->u8>>::f"
        );
        assert_eq!(key_of_impl("impl Foo<{ N + 1 }>"), "Foo<{N+1}>::f");
    }

    #[test]
    fn the_names_of_the_generics_of_an_impl_are_in_the_key() {
        assert_ne!(key_of_impl("impl<T> Foo<T>"), key_of_impl("impl<U> Foo<U>"));
    }

    #[test]
    fn a_source_that_does_not_parse_is_refused_and_so_is_another_protocol() {
        assert!(matches!(
            items(&Request {
                protocol: PROTOCOL,
                source: "fn (".to_string()
            }),
            Err(Error::Source(_))
        ));
        assert!(matches!(
            items(&Request {
                protocol: 2,
                source: String::new()
            }),
            Err(Error::Protocol(2))
        ));
        assert!(matches!(handle(b"not json"), Err(Error::Request(_))));
    }

    #[test]
    fn the_reply_is_the_json_of_the_protocol() {
        let out = handle(br#"{"protocol":1,"source":"fn price(total: u64) -> u64;\n"}"#).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"{"protocol":1,"items":[{"key":"price","lines":[1,1]}]}"#
        );
    }

    #[test]
    fn the_same_request_gives_the_same_reply_byte_for_byte() {
        let request = br#"{"protocol":1,"source":"mod m { impl<T> Foo<T> { fn f() {} } }\n"}"#;
        assert_eq!(handle(request).unwrap(), handle(request).unwrap());
    }

    #[test]
    fn an_empty_source_has_no_item() {
        assert!(ask("").is_empty());
        assert!(ask("// nothing here\n").is_empty());
    }
}
