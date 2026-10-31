//! The Rust provider of Items. It speaks `chidev-items 2`: one JSON request `{protocol, source}` in, one JSON reply
//! `{protocol, items: [{key, lines}], left_out}` out, and nothing else. Like the provider of mutants it is a pure
//! function of the request: it touches no file, no process and no clock.
//!
//! A **caller** of a source is code outside that source, built as a caller builds it (not as part of the source's
//! own tests). An Item is a declaration such a caller can name. The provider answers the Items, and in `left_out`
//! how many declarations it found that are not. `left_out` is always written, even when it is 0.
//!
//! An Item is found by its **key** and the **lines** it is declared on. The lines
//! point at the declaration alone: from its first token (after the doc comments and attributes) to the line that
//! ends the signature, with the `;` or the `{` of its body. A `struct`, `enum` or `union` declares its fields or
//! variants, so its lines go on to the closing `}`: a field added or retyped is a change of what the Item declares.
//! Two declarations on one line have lines that overlap, and that is a right answer.
//!
//! # What is found
//!
//! A `fn`, `struct`, `enum`, `union`, `trait`, `type`, `const` and `static` item, a `macro_rules!` (its key is
//! `name!`), and the `fn`, `const` and `type` inside an `impl` or a `trait`. A declaration with no body
//! (`fn f() -> u64;`, which an Interface holds when it is the code a Case is written against) is not an item to
//! the Rust parser, so it is parsed again as a declaration; tokens that cannot be read as one are counted in
//! `left_out`, since the provider cannot say that a caller can name them. Nothing else is found: not a `mod`
//! itself (an inline one is entered), not an `impl` block, not what is inside a function body, not a `use`.
//!
//! # What a caller can name
//!
//! Which found declaration is an Item is written down once, as the table of the README under "Provider protocol
//! `chidev-items 2`"; this module is that table as code and does not copy it. A found declaration is an Item when it
//! reaches "outside" (outside the module of the file and the inline `mod`s in it), and counts in `left_out` when it
//! does not. A declaration with no body is judged by the same rules.
//!
//! The walk carries what the place it is in allows: `outside` (every inline `mod` around it reaches) and `test`
//! (one of them is `#[cfg(test)]`). `Walk::allowed` joins that with the attributes of the declaration (test code is
//! never an Item) and, where it has one of its own, its visibility (`reaches`). A member of an `impl` is judged by
//! the type and the trait the `impl` is for, not by the `mod` it is written in: `Walk::declare` first notes which
//! types and traits the file declares and whether each reaches, and `Walk::path_reaches` looks one up in the module
//! the `impl` is in, by the path as written (`self::`, `super::` and the names of inline `mod`s are followed; a path
//! from `crate` or from another file is not found, so it reaches). A `macro_rules!` has no visibility, so it reaches
//! with `#[macro_export]` outside test code, whatever `mod` it is in.
//!
//! A `cfg` condition that cannot be read is not guessed at: it makes the whole request fail with [`Error::Source`],
//! as a source that does not parse does.
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

use std::collections::BTreeMap;

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use syn::ext::IdentExt;
use syn::parse::discouraged::Speculative;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use thiserror::Error;

/// The protocol version this provider speaks.
pub const PROTOCOL: u32 = 2;

/// What chidev asks for: the Items of `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The protocol version; this provider answers only [`PROTOCOL`].
    pub protocol: u32,
    /// The code of the Interface.
    pub source: String,
}

/// One declaration a caller can name.
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
    /// The declarations a caller can name, in the order of the source: a `trait` or a `mod`'s item before what is
    /// inside it.
    pub items: Vec<Item>,
    /// How many declarations were found that a caller cannot name. Always written, 0 included.
    pub left_out: usize,
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
    let mut walk = Walk::new();
    walk.declare(&file.items)?;
    walk.items(&file.items)?;
    Ok(Reply {
        protocol: PROTOCOL,
        items: walk.found,
        left_out: walk.left_out,
    })
}

/// Walks the declarations of a file and keeps the Items.
struct Walk {
    /// The `mod`s the walk is inside, outermost first.
    path: Vec<String>,
    /// Whether the `mod`s the walk is inside all reach outside (and none is under `#[cfg(test)]`).
    outside: bool,
    /// Whether one of the `mod`s the walk is inside is test code (`#[cfg(test)]`): what is in it builds for a test
    /// only, wherever it is named from.
    test: bool,
    /// The types and traits the file declares, by their path from the file, and whether each reaches outside.
    declared: BTreeMap<Vec<String>, bool>,
    found: Vec<Item>,
    /// The declarations found that are not Items.
    left_out: usize,
}

impl Walk {
    fn new() -> Walk {
        Walk {
            path: Vec::new(),
            outside: true,
            test: false,
            declared: BTreeMap::new(),
            found: Vec::new(),
            left_out: 0,
        }
    }

    /// Notes which types and traits the file declares and whether each reaches outside, before any `impl` is read.
    fn declare(&mut self, items: &[syn::Item]) -> Result<(), Error> {
        for item in items {
            let (name, attrs, vis) = match item {
                syn::Item::Struct(s) => (&s.ident, &s.attrs, &s.vis),
                syn::Item::Enum(e) => (&e.ident, &e.attrs, &e.vis),
                syn::Item::Union(u) => (&u.ident, &u.attrs, &u.vis),
                syn::Item::Type(t) => (&t.ident, &t.attrs, &t.vis),
                syn::Item::Trait(t) => (&t.ident, &t.attrs, &t.vis),
                // a type with no body (`type Money;`) is a type the file declares too
                syn::Item::Verbatim(tokens) => {
                    match syn::parse2::<Bodyless>(tokens.clone()) {
                        Ok(d) if d.is_type => {
                            let reaches = self.allowed(self.outside, &d.attrs, Some(&d.vis))?;
                            self.note(&d.name, reaches);
                        }
                        // not a type; or not a declaration at all, which `Walk::declared` counts in `left_out`
                        Ok(_) | Err(_) => {}
                    }
                    continue;
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.inside(m, |walk| walk.declare(inner))?;
                    }
                    continue;
                }
                _ => continue,
            };
            let reaches = self.allowed(self.outside, attrs, Some(vis))?;
            self.note(&name.to_string(), reaches);
        }
        Ok(())
    }

    /// Notes that the file declares the type or trait `name` in the `mod` the walk is in, reaching outside or not.
    fn note(&mut self, name: &str, reaches: bool) {
        let mut full = self.path.clone();
        full.push(name.to_string());
        *self.declared.entry(full).or_insert(false) |= reaches;
    }

    /// Runs `f` inside the inline `mod` `m`: its name leads the path, and what it allows is what the walk allows.
    fn inside(
        &mut self,
        m: &syn::ItemMod,
        f: impl FnOnce(&mut Walk) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let (outside, test) = (self.outside, self.test);
        self.outside = self.allowed(outside, &m.attrs, Some(&m.vis))?;
        self.test = test || is_test(&m.attrs)?;
        self.path.push(m.ident.to_string());
        let walked = f(self);
        self.path.pop();
        (self.outside, self.test) = (outside, test);
        walked
    }

    /// Whether a declaration reaches outside: `base` (what the place it is in allows), no `#[cfg(test)]` or
    /// `#[test]` on it, and its own visibility `vis` (none for a member whose visibility is not its own, a trait's or
    /// a trait impl's). The attributes are read whatever `base` is, so that a `cfg` that cannot be read is always
    /// an error.
    fn allowed(
        &self,
        base: bool,
        attrs: &[syn::Attribute],
        vis: Option<&syn::Visibility>,
    ) -> Result<bool, Error> {
        let test = is_test(attrs)?;
        Ok(base && !test && vis.is_none_or(|v| reaches(v, self.path.len())))
    }

    /// Whether the type or trait named by `path` reaches outside: what the file declares under that path, or
    /// reaching when it declares none (the name is from another file).
    fn path_reaches(&self, path: &syn::Path) -> bool {
        let mut full = self.path.clone();
        if path.leading_colon.is_some() {
            return true;
        }
        for (n, segment) in path.segments.iter().enumerate() {
            let word = segment.ident.to_string();
            match word.as_str() {
                "crate" => return true,
                "self" if n == 0 => {}
                "super" => {
                    if full.pop().is_none() {
                        return true;
                    }
                }
                _ => full.push(word),
            }
        }
        self.declared.get(&full).copied().unwrap_or(true)
    }

    /// Whether the type `ty` of an `impl` reaches outside: a path is looked up (past references, pointers, slices
    /// and arrays; a tuple when each of its elements does), any other type reaches.
    fn type_reaches(&self, ty: &syn::Type) -> bool {
        match ty {
            syn::Type::Path(p) if p.qself.is_none() => self.path_reaches(&p.path),
            syn::Type::Reference(r) => self.type_reaches(&r.elem),
            syn::Type::Paren(p) => self.type_reaches(&p.elem),
            syn::Type::Group(g) => self.type_reaches(&g.elem),
            syn::Type::Slice(s) => self.type_reaches(&s.elem),
            syn::Type::Array(a) => self.type_reaches(&a.elem),
            syn::Type::Ptr(p) => self.type_reaches(&p.elem),
            syn::Type::Tuple(t) => t.elems.iter().all(|e| self.type_reaches(e)),
            _ => true,
        }
    }

    fn items(&mut self, items: &[syn::Item]) -> Result<(), Error> {
        for item in items {
            self.item(item)?;
        }
        Ok(())
    }

    /// Keeps the declaration `name` of the scope `owner` (a type, a trait; none for a free item) when its tokens run
    /// from their first one (past the attributes) to `end`: an Item when `reach`, otherwise it is counted as left out.
    fn keep(
        &mut self,
        owner: Option<&str>,
        name: &str,
        tokens: TokenStream,
        end: Span,
        reach: bool,
    ) {
        let Some(first) = first_line(tokens) else {
            return;
        };
        if !reach {
            self.left_out += 1;
            return;
        }
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

    /// Keeps the free declaration `name` with these attributes and visibility, as [`Walk::keep`] does: an Item when
    /// it reaches outside from the `mod` the walk is in.
    fn free(
        &mut self,
        attrs: &[syn::Attribute],
        vis: &syn::Visibility,
        name: &str,
        tokens: TokenStream,
        end: Span,
    ) -> Result<(), Error> {
        let reach = self.allowed(self.outside, attrs, Some(vis))?;
        self.keep(None, name, tokens, end, reach);
        Ok(())
    }

    fn item(&mut self, item: &syn::Item) -> Result<(), Error> {
        match item {
            syn::Item::Fn(f) => self.free(
                &f.attrs,
                &f.vis,
                &f.sig.ident.to_string(),
                f.to_token_stream(),
                f.block.brace_token.span.open(),
            ),
            // The fields and variants of a type are what it declares, so its lines go to the closing brace.
            syn::Item::Struct(s) => {
                let end = match &s.fields {
                    syn::Fields::Named(named) => named.brace_token.span.close(),
                    _ => semi_or(s.semi_token.as_ref(), s.ident.span()),
                };
                self.free(
                    &s.attrs,
                    &s.vis,
                    &s.ident.to_string(),
                    s.to_token_stream(),
                    end,
                )
            }
            syn::Item::Enum(e) => self.free(
                &e.attrs,
                &e.vis,
                &e.ident.to_string(),
                e.to_token_stream(),
                e.brace_token.span.close(),
            ),
            syn::Item::Union(u) => self.free(
                &u.attrs,
                &u.vis,
                &u.ident.to_string(),
                u.to_token_stream(),
                u.fields.brace_token.span.close(),
            ),
            syn::Item::Type(t) => self.free(
                &t.attrs,
                &t.vis,
                &t.ident.to_string(),
                t.to_token_stream(),
                t.semi_token.span(),
            ),
            syn::Item::Const(c) => self.free(
                &c.attrs,
                &c.vis,
                &c.ident.to_string(),
                c.to_token_stream(),
                c.semi_token.span(),
            ),
            syn::Item::Static(s) => self.free(
                &s.attrs,
                &s.vis,
                &s.ident.to_string(),
                s.to_token_stream(),
                s.semi_token.span(),
            ),
            // A macro has no visibility: `#[macro_export]` puts it at the root of the crate, whatever `mod` it is
            // written in, so only test code keeps it from a caller.
            syn::Item::Macro(m) if m.mac.path.is_ident("macro_rules") => {
                if let Some(name) = &m.ident {
                    let exported = m.attrs.iter().any(|a| a.path().is_ident("macro_export"));
                    let reach = self.allowed(exported && !self.test, &m.attrs, None)?;
                    self.keep(
                        None,
                        &format!("{name}!"),
                        m.to_token_stream(),
                        m.mac.delimiter.span().open(),
                        reach,
                    );
                }
                Ok(())
            }
            syn::Item::Trait(t) => {
                let reach = self.allowed(self.outside, &t.attrs, Some(&t.vis))?;
                self.keep(
                    None,
                    &t.ident.to_string(),
                    t.to_token_stream(),
                    t.brace_token.span.open(),
                    reach,
                );
                let owner = self.scoped(&t.ident.to_string());
                for member in &t.items {
                    self.trait_item(&owner, member, reach)?;
                }
                Ok(())
            }
            // An `impl` gives its members to the type wherever it is written, so the `mod` it is in does not keep
            // them from a caller (test code does): the type, the trait and the members' own visibility do.
            syn::Item::Impl(i) => {
                let owner = self.impl_owner(i);
                let types = self.type_reaches(&i.self_ty);
                let (base, own) = match &i.trait_ {
                    // an inherent `impl T`: each member's visibility, when `T` reaches
                    None => (types, true),
                    // an `impl Trait for T`: no member has a visibility, and `Trait` and `T` both have to reach
                    Some((_, path, _)) => (types && self.path_reaches(path), false),
                };
                let base = self.allowed(!self.test && base, &i.attrs, None)?;
                for member in &i.items {
                    self.impl_item(&owner, member, base, own)?;
                }
                Ok(())
            }
            syn::Item::Mod(m) => match &m.content {
                Some((_, items)) => self.inside(m, |walk| walk.items(items)),
                None => Ok(()),
            },
            syn::Item::Verbatim(tokens) => self.declared(None, tokens.clone(), self.outside, true),
            _ => Ok(()),
        }
    }

    /// A member of a trait: an Item when the trait is (`trait_reach`), whatever it says of itself.
    fn trait_item(
        &mut self,
        owner: &str,
        member: &syn::TraitItem,
        trait_reach: bool,
    ) -> Result<(), Error> {
        match member {
            syn::TraitItem::Fn(f) => {
                let end = match &f.default {
                    Some(block) => block.brace_token.span.open(),
                    None => semi_or(f.semi_token.as_ref(), f.sig.ident.span()),
                };
                let reach = self.allowed(trait_reach, &f.attrs, None)?;
                self.keep(
                    Some(owner),
                    &f.sig.ident.to_string(),
                    f.to_token_stream(),
                    end,
                    reach,
                );
                Ok(())
            }
            syn::TraitItem::Const(c) => {
                let reach = self.allowed(trait_reach, &c.attrs, None)?;
                self.keep(
                    Some(owner),
                    &c.ident.to_string(),
                    c.to_token_stream(),
                    c.semi_token.span(),
                    reach,
                );
                Ok(())
            }
            syn::TraitItem::Type(t) => {
                let reach = self.allowed(trait_reach, &t.attrs, None)?;
                self.keep(
                    Some(owner),
                    &t.ident.to_string(),
                    t.to_token_stream(),
                    t.semi_token.span(),
                    reach,
                );
                Ok(())
            }
            syn::TraitItem::Verbatim(tokens) => {
                self.declared(Some(owner), tokens.clone(), trait_reach, false)
            }
            _ => Ok(()),
        }
    }

    /// A member of an `impl`: an Item when the `impl` lets it (`base`) and, for an inherent `impl` (`own`), when its
    /// own visibility reaches.
    fn impl_item(
        &mut self,
        owner: &str,
        member: &syn::ImplItem,
        base: bool,
        own: bool,
    ) -> Result<(), Error> {
        match member {
            syn::ImplItem::Fn(f) => {
                let reach = self.allowed(base, &f.attrs, own.then_some(&f.vis))?;
                self.keep(
                    Some(owner),
                    &f.sig.ident.to_string(),
                    f.to_token_stream(),
                    f.block.brace_token.span.open(),
                    reach,
                );
                Ok(())
            }
            syn::ImplItem::Const(c) => {
                let reach = self.allowed(base, &c.attrs, own.then_some(&c.vis))?;
                self.keep(
                    Some(owner),
                    &c.ident.to_string(),
                    c.to_token_stream(),
                    c.semi_token.span(),
                    reach,
                );
                Ok(())
            }
            syn::ImplItem::Type(t) => {
                let reach = self.allowed(base, &t.attrs, own.then_some(&t.vis))?;
                self.keep(
                    Some(owner),
                    &t.ident.to_string(),
                    t.to_token_stream(),
                    t.semi_token.span(),
                    reach,
                );
                Ok(())
            }
            syn::ImplItem::Verbatim(tokens) => {
                self.declared(Some(owner), tokens.clone(), base, own)
            }
            _ => Ok(()),
        }
    }

    /// A declaration with no body, which the parser leaves as tokens (`pub fn f() -> u64;`, `const N: usize;`):
    /// parsed again as the declaration it is, and judged as any other (`base` is what the place it is in allows;
    /// `own` says that its own visibility counts). Tokens that cannot be read as one are counted in `left_out`: the
    /// provider cannot say that a caller can name them, and does not drop them silently.
    fn declared(
        &mut self,
        owner: Option<&str>,
        tokens: TokenStream,
        base: bool,
        own: bool,
    ) -> Result<(), Error> {
        let Ok(d) = syn::parse2::<Bodyless>(tokens) else {
            self.left_out += 1;
            return Ok(());
        };
        if !self.allowed(base, &d.attrs, own.then_some(&d.vis))? {
            self.left_out += 1;
            return Ok(());
        }
        let key = match owner {
            Some(owner) => format!("{owner}::{}", d.name),
            None => self.scoped(&d.name),
        };
        self.found.push(Item {
            key,
            lines: [d.first, d.end.max(d.first)],
        });
        Ok(())
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

/// Whether a declaration with these attributes is test code: `#[test]`, or `#[cfg(test)]` (also `all(test, ..)`, and
/// `any(..)` of which every branch is, since those build only for a test). A `cfg` argument that cannot be read is an
/// [`Error::Source`], not a guess that it is not a test: a guess could make test code an Item.
fn is_test(attrs: &[syn::Attribute]) -> Result<bool, Error> {
    let mut test = false;
    for a in attrs {
        // `#[test]`, and a test attribute of a crate (`#[tokio::test]`)
        if a.path().segments.last().is_some_and(|s| s.ident == "test") {
            test = true;
        } else if a.path().is_ident("cfg") {
            let meta = a.parse_args::<syn::Meta>().map_err(cfg_unreadable)?;
            test |= cfg_is_test(&meta)?;
        }
    }
    Ok(test)
}

/// Whether a `cfg` condition holds only when building for a test.
fn cfg_is_test(meta: &syn::Meta) -> Result<bool, Error> {
    match meta {
        syn::Meta::Path(p) => Ok(p.is_ident("test")),
        syn::Meta::List(l) if l.path.is_ident("all") || l.path.is_ident("any") => {
            let inner = l
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .map_err(cfg_unreadable)?;
            let tests = inner
                .iter()
                .map(cfg_is_test)
                .collect::<Result<Vec<bool>, Error>>()?;
            Ok(if l.path.is_ident("all") {
                tests.iter().any(|t| *t)
            } else {
                !tests.is_empty() && tests.iter().all(|t| *t)
            })
        }
        _ => Ok(false),
    }
}

/// The failure of a `cfg` condition that cannot be read.
fn cfg_unreadable(e: syn::Error) -> Error {
    Error::Source(format!("a `cfg` condition cannot be read: {e}"))
}

/// Whether a visibility written at `depth` inline `mod`s deep (0 at the top of the file) reaches outside the file's
/// module and its inline `mod`s.
fn reaches(vis: &syn::Visibility, depth: usize) -> bool {
    match vis {
        syn::Visibility::Inherited => false,
        syn::Visibility::Public(_) => true,
        syn::Visibility::Restricted(r) => {
            if r.in_token.is_some() {
                // `self` stays inside; `super` as many times as there are `mod`s around and once more leaves the
                // file's module (`pub(in super)` is `pub(super)`); any other path is an approximation, as the
                // provider does not know the module path
                let mut segments = r
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .peekable();
                if segments.peek().is_some_and(|s| s == "self") {
                    false
                } else if segments.peek().is_some_and(|s| s == "super") {
                    segments.take_while(|s| s == "super").count() > depth
                } else {
                    true
                }
            } else if r.path.is_ident("self") {
                false
            } else if r.path.is_ident("super") {
                // the parent of an inline `mod` is the file's module or another inline `mod`
                depth == 0
            } else {
                true
            }
        }
    }
}

/// A declaration that has no body, as it is parsed again: its name, whether it declares a type, its attributes and
/// visibility, the line of its first token past the attributes, and the line of its `;`.
struct Bodyless {
    name: String,
    is_type: bool,
    attrs: Vec<syn::Attribute>,
    vis: syn::Visibility,
    first: usize,
    end: usize,
}

impl Parse for Bodyless {
    fn parse(input: ParseStream) -> syn::Result<Bodyless> {
        let attrs = input.call(syn::Attribute::parse_outer)?;
        let first = input.cursor().span().start().line;
        let vis = input.parse::<syn::Visibility>()?;
        input.parse::<Option<syn::Token![default]>>()?;
        // `const fn f();` is a function and `const N: usize;` is not: a signature is tried first
        let ahead = input.fork();
        let is_type = input.peek(syn::Token![type]);
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
        Ok(Bodyless {
            name,
            is_type,
            attrs,
            vis,
            first,
            end,
        })
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

    /// The key of the one `fn f` in an `impl` with the header `header` (`pub`, which the inherent ones need and a
    /// trait's do not read).
    fn key_of_impl(header: &str) -> String {
        let source = format!("{header} {{ pub fn f() {{}} }}");
        let all = keys(&source);
        assert_eq!(all.len(), 1, "{header}: {all:?}");
        all.into_iter().next().unwrap()
    }

    #[test]
    fn a_free_fn_or_type_is_its_name_and_a_mod_leads_it_with_its_path() {
        let source = "pub fn a() {}\npub struct B;\npub mod m {\n    pub fn c() {}\n    pub mod n {\n        pub enum D {}\n    }\n}\npub const E: u8 = 1;\n";
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

pub struct Unit;
pub struct Tuple(
    u8,
);
pub struct Named {
    n: u8,
}
pub enum Kind {
    A,
}
pub union Bits {
    a: u8,
}
pub trait Shape {
    fn area(&self) -> u64;
}
pub type Alias = u8;
pub const LIMIT: u8 = 1;
pub static COUNT: u8 = 1;
#[macro_export]
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
                ("Named".to_string(), [13, 15]),
                ("Kind".to_string(), [16, 18]),
                ("Bits".to_string(), [19, 21]),
                ("Shape".to_string(), [22, 22]),
                ("Shape::area".to_string(), [23, 23]),
                ("Alias".to_string(), [25, 25]),
                ("LIMIT".to_string(), [26, 26]),
                ("COUNT".to_string(), [27, 27]),
                ("twice!".to_string(), [29, 29]),
            ]
        );
    }

    #[test]
    fn the_lines_start_after_the_doc_comments_and_attributes() {
        let source =
            "/// One.\n/// Two.\n#[must_use]\n#[cfg(unix)]\npub fn f() -> u8 {\n    1\n}\n";
        assert_eq!(found(source), [("f".to_string(), [5, 5])]);
    }

    #[test]
    fn the_lines_end_at_the_line_of_the_semicolon_or_the_brace_of_the_body() {
        let source = "pub fn f<T>(x: T)\nwhere\n    T: Copy,\n{\n    let _ = x;\n}\npub type Long =\n    u8;\n";
        assert_eq!(
            found(source),
            [("f".to_string(), [1, 4]), ("Long".to_string(), [7, 8])]
        );
    }

    #[test]
    fn a_type_with_fields_or_variants_goes_on_to_its_closing_brace() {
        let source = "pub struct P {\n    x: u8,\n}\npub enum Kind {\n    A,\n    B,\n}\npub union U {\n    a: u8,\n}\npub struct T(u8);\n";
        assert_eq!(
            found(source),
            [
                ("P".to_string(), [1, 3]),
                ("Kind".to_string(), [4, 7]),
                ("U".to_string(), [8, 10]),
                ("T".to_string(), [11, 11]),
            ]
        );
    }

    #[test]
    fn members_of_an_impl_and_a_trait_are_found_whatever_their_kind() {
        let source = "\
pub trait Tr<T> {
    type Out;
    const N: usize;
    fn make() -> T;
    fn name(&self) -> u8 { 1 }
}
pub struct S;
impl S {
    pub const K: u8 = 1;
    pub fn new() -> S { S }
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
        let source = "pub mod m {\n    pub trait Tr { fn a(); }\n    pub struct S;\n    impl S { pub fn b() {} }\n    impl Tr for S { fn a() {} }\n}\n";
        assert_eq!(
            keys(source),
            ["m::Tr", "m::Tr::a", "m::S", "m::S::b", "m::<S as Tr>::a"]
        );
    }

    #[test]
    fn what_a_key_does_not_name_is_not_found() {
        // a `mod` itself, an `impl` block, a `use`, a macro call, and what a function body holds
        let source = "mod outer;\nmod inner {}\nuse std::fmt;\nthread_local! { static X: u8 = 1; }\npub fn f() {\n    fn nested() {}\n    struct Local;\n}\nimpl Drop for S { fn drop(&mut self) {} }\npub struct S;\nextern \"C\" {\n    fn c();\n}\n";
        assert_eq!(keys(source), ["f", "<S as Drop>::drop", "S"]);
    }

    #[test]
    fn the_inherent_impls_of_one_type_with_other_arguments_are_other_keys() {
        let source = "impl Foo<u8> { pub fn f() {} }\nimpl Foo<u16> { pub fn f() {} }\nimpl<T> Foo<T> { pub fn f() {} }\n";
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
pub const fn square(n: u64) -> u64;
pub unsafe extern \"C\" fn raw(
    p: *const u8,
) -> u8;
pub const LIMIT: u64;
pub static COUNT: u64;
pub type Money;
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
        let source = "pub mod m {\n    pub fn f() -> u8;\n}\nimpl S {\n    pub fn g() -> u8;\n}\n";
        assert_eq!(keys(source), ["m::f", "S::g"]);
    }

    #[test]
    fn two_declarations_on_one_line_have_lines_that_overlap() {
        let source = "pub struct A; pub struct B;\npub trait Clock { fn now(&self) -> u64; }\n";
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
        assert_eq!(keys("pub struct Foo;\npub fn Foo() {}\n"), ["Foo", "Foo"]);
        assert_eq!(
            keys("#[cfg(unix)]\npub fn f() {}\n#[cfg(windows)]\npub fn f() {}\n"),
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
        let broken = "impl<A, B> Foo<\n    A,\n    B,\n> {\n    pub fn f() {}\n}\n";
        let plain = "impl<A, B> Foo<A, B> { pub fn f() {} }";
        assert_eq!(keys(broken), ["Foo<A,B>::f"]);
        assert_eq!(keys(broken), keys(plain));
        let tupled = "impl Foo<(\n    A,\n    B,\n)> { pub fn f() {} }";
        assert_eq!(keys(tupled), ["Foo<(A,B)>::f"]);
        let function = "impl Foo<fn(\n    A,\n    B,\n) -> C> { pub fn f() {} }";
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
                protocol: 1,
                source: String::new()
            }),
            Err(Error::Protocol(1))
        ));
        assert!(matches!(handle(b"not json"), Err(Error::Request(_))));
    }

    #[test]
    fn the_reply_is_the_json_of_the_protocol_and_left_out_is_always_in_it() {
        let out =
            handle(br#"{"protocol":2,"source":"pub fn price(total: u64) -> u64;\n"}"#).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"{"protocol":2,"items":[{"key":"price","lines":[1,1]}],"left_out":0}"#
        );
        let out = handle(br#"{"protocol":2,"source":"fn helper() {}\n"}"#).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"{"protocol":2,"items":[],"left_out":1}"#
        );
    }

    #[test]
    fn the_same_request_gives_the_same_reply_byte_for_byte() {
        let request =
            br#"{"protocol":2,"source":"pub mod m { impl<T> Foo<T> { pub fn f() {} } }\n"}"#;
        assert_eq!(handle(request).unwrap(), handle(request).unwrap());
    }

    #[test]
    fn an_empty_source_has_no_item() {
        assert!(ask("").is_empty());
        assert!(ask("// nothing here\n").is_empty());
        assert_eq!(left_out(""), 0);
    }

    /// How many declarations of `source` the provider found that a caller cannot name.
    fn left_out(source: &str) -> usize {
        items(&Request {
            protocol: PROTOCOL,
            source: source.to_string(),
        })
        .unwrap_or_else(|e| panic!("no reply for {source:?}: {e}"))
        .left_out
    }

    #[test]
    fn what_a_caller_cannot_name_is_not_an_item_and_is_counted() {
        let source = "\
pub fn api() {}
fn helper() {}
mod inner { pub fn x() {} }
mod m { pub(super) fn y() {} }
pub(super) fn z() {}
#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
";
        // helper, inner::x (inner is private), m::y (`pub(super)` reaches the file's module only) and tests::t
        assert_eq!(keys(source), ["api", "z"]);
        assert_eq!(left_out(source), 4);
    }

    #[test]
    fn the_visibility_that_reaches_outside_is_pub_pub_crate_and_pub_super_at_the_top() {
        let source = "\
fn a() {}
pub(self) fn b() {}
pub fn c() {}
pub(crate) fn d() {}
pub(super) fn e() {}
pub mod m {
    pub(super) fn f() {}
    pub(self) fn g() {}
    pub(crate) fn h() {}
    pub mod n {
        pub(super) fn i() {}
        pub fn j() {}
    }
}
";
        // left out: a, b, m::f, m::g, m::n::i
        assert_eq!(keys(source), ["c", "d", "e", "m::h", "m::n::j"]);
        assert_eq!(left_out(source), 5);
    }

    #[test]
    fn pub_in_a_path_is_out_when_the_path_starts_with_self_and_in_otherwise() {
        assert!(keys("pub(in self::x) fn a() {}").is_empty());
        assert!(keys("pub(in self) fn a() {}").is_empty());
        assert_eq!(left_out("pub(in self::x) fn a() {}"), 1);
        assert_eq!(keys("pub(in crate::a) fn a() {}"), ["a"]);
        assert_eq!(keys("pub(in super::a) fn a() {}"), ["a"]);
    }

    #[test]
    fn a_declaration_in_a_mod_needs_every_mod_around_it_to_reach_too() {
        let source = "pub mod a {\n    mod b {\n        pub fn hidden() {}\n    }\n    pub mod c {\n        pub fn shown() {}\n        fn private() {}\n    }\n}\n";
        assert_eq!(keys(source), ["a::c::shown"]);
        assert_eq!(left_out(source), 2);
    }

    #[test]
    fn a_member_of_an_inherent_impl_of_a_type_that_does_not_reach_is_not_an_item() {
        let source = "struct S;\nimpl S { pub fn f() {} }\npub struct P;\nimpl P { pub fn g() {} fn h() {} }\nimpl Elsewhere { pub fn i() {} }\n";
        // left out: S, S::f, P::h
        assert_eq!(keys(source), ["P", "P::g", "Elsewhere::i"]);
        assert_eq!(left_out(source), 3);
        // the type is looked up where the `impl` is, by the path as written
        let source = "mod m {\n    pub struct S;\n}\npub struct S;\nimpl S { pub fn f() {} }\nimpl m::S { pub fn g() {} }\n";
        // left out: m::S (m is private), m::S::g
        assert_eq!(keys(source), ["S", "S::f"]);
        assert_eq!(left_out(source), 2);
    }

    #[test]
    fn a_member_of_a_trait_is_an_item_when_the_trait_is() {
        let source = "pub trait A { fn a(); const N: u8; type T; }\ntrait B { fn b(); }\n";
        assert_eq!(keys(source), ["A", "A::a", "A::N", "A::T"]);
        assert_eq!(left_out(source), 2);
    }

    #[test]
    fn a_member_of_a_trait_impl_is_an_item_when_the_trait_and_the_type_declared_here_reach() {
        let source = "pub struct P;\nimpl std::fmt::Display for P {\n    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) }\n}\n";
        assert_eq!(keys(source), ["P", "<P as std::fmt::Display>::fmt"]);
        assert_eq!(left_out(source), 0);
        // a type declared here that does not reach
        let source = "struct S;\nimpl std::fmt::Display for S { fn fmt() {} }\n";
        assert!(keys(source).is_empty());
        assert_eq!(left_out(source), 2);
        // a trait declared here that does not reach, for a type that does
        let source = "trait T { fn t(); }\npub struct P;\nimpl T for P { fn t() {} }\nimpl Elsewhere for P { fn e() {} }\n";
        assert_eq!(keys(source), ["P", "<P as Elsewhere>::e"]);
        assert_eq!(left_out(source), 3);
        // names from another file reach
        assert_eq!(
            keys("impl Other for Another { fn f() {} }"),
            ["<Another as Other>::f"]
        );
        // through a reference
        let source = "struct S;\nimpl Elsewhere for &S { fn f() {} }\n";
        assert!(keys(source).is_empty());
    }

    #[test]
    fn test_code_is_not_an_item_wherever_it_is() {
        let source = "\
#[cfg(test)]
pub fn a() {}
#[test]
pub fn b() {}
#[cfg(all(test, unix))]
pub fn c() {}
#[cfg(not(test))]
pub fn d() {}
#[cfg(unix)]
pub fn e() {}
#[cfg(any(test, unix))]
pub fn f() {}
#[cfg(test)]
pub struct Fake;
impl Fake { pub fn g() {} }
pub struct S;
#[cfg(test)]
impl S { pub fn h() {} }
impl S {
    #[cfg(test)]
    pub fn i() {}
    pub fn j() {}
}
pub trait Tr {
    #[cfg(test)]
    fn k();
    fn l();
}
#[cfg(test)]
pub mod tests {
    pub fn m() {}
}
";
        assert_eq!(keys(source), ["d", "e", "f", "S", "S::j", "Tr", "Tr::l"]);
        // a, b, c, Fake, Fake::g, h, i, k, m
        assert_eq!(left_out(source), 9);
    }

    #[test]
    fn a_declaration_with_no_body_is_judged_by_the_same_rules() {
        let source = "\
pub fn a() -> u8;
fn b() -> u8;
pub(super) fn c() -> u8;
#[cfg(test)]
pub fn d() -> u8;
mod m {
    pub fn e() -> u8;
}
pub mod n {
    pub(super) fn f() -> u8;
    pub fn g() -> u8;
}
struct S;
impl S {
    pub fn h() -> u8;
}
pub struct P;
impl P {
    pub fn i() -> u8;
    fn j() -> u8;
}
pub trait Tr {
    const N: usize;
}
trait Hidden {
    const M: usize;
}
";
        assert_eq!(keys(source), ["a", "c", "n::g", "P", "P::i", "Tr", "Tr::N"]);
        // b, d, m::e, n::f, S, S::h, P::j, Hidden, Hidden::M
        assert_eq!(left_out(source), 9);
    }

    #[test]
    fn an_impl_gives_its_members_to_the_type_whatever_mod_it_is_written_in() {
        let source = "pub struct S;\nmod m {\n    impl super::S { pub fn f() {} fn g() {} }\n    impl std::fmt::Debug for super::S { fn fmt() {} }\n}\n#[cfg(test)]\nmod tests {\n    impl super::S { pub fn h() {} }\n}\n";
        // left out: m::super::S::g (no visibility), and h (test code)
        assert_eq!(
            keys(source),
            [
                "S",
                "m::super::S::f",
                "m::<super::S as std::fmt::Debug>::fmt"
            ]
        );
        assert_eq!(left_out(source), 2);
    }

    #[test]
    fn pub_in_super_is_pub_super_and_reaches_only_when_it_climbs_out_of_the_file() {
        let source = "pub mod m {\n    pub(in super) fn a() {}\n    pub(in super::super) fn b() {}\n    pub(in super::x) fn c() {}\n}\npub(in super) fn d() {}\n";
        assert_eq!(keys(source), ["m::b", "d"]);
        assert_eq!(left_out(source), 2);
    }

    #[test]
    fn a_test_attribute_of_a_crate_is_test_code_too() {
        let source = "#[tokio::test]\npub async fn t() {}\n";
        assert!(keys(source).is_empty());
        assert_eq!(left_out(source), 1);
    }

    #[test]
    fn a_private_type_with_no_body_or_inside_another_type_keeps_its_impl_from_a_caller() {
        let source = "type Money;\nimpl Money { pub fn f() -> u8; }\nstruct S;\nimpl Tr for [S] { fn g() {} }\nimpl Tr for (u8, S) { fn h() {} }\nimpl Tr for *const S { fn i() {} }\n";
        assert!(keys(source).is_empty());
        // Money, Money::f, S, g, h, i
        assert_eq!(left_out(source), 6);
    }

    #[test]
    fn a_macro_is_an_item_only_when_exported() {
        let source =
            "#[macro_export]\nmacro_rules! a { () => {}; }\nmacro_rules! b { () => {}; }\n";
        assert_eq!(keys(source), ["a!"]);
        assert_eq!(left_out(source), 1);
        // exported from a private `mod` all the same, but not from test code
        let source = "mod m {\n    #[macro_export]\n    macro_rules! a { () => {}; }\n}\n#[cfg(test)]\nmod tests {\n    #[macro_export]\n    macro_rules! b { () => {}; }\n}\n";
        assert_eq!(keys(source), ["m::a!"]);
        assert_eq!(left_out(source), 1);
    }

    #[test]
    fn a_declaration_the_parser_leaves_as_tokens_and_that_cannot_be_read_is_left_out_not_dropped() {
        // a `macro` item is tokens to the parser, and not a declaration with no body
        let source = "pub fn a() {}\npub macro m() {}\n";
        assert_eq!(keys(source), ["a"]);
        assert_eq!(left_out(source), 1);
        // the same inside a `mod`, where it is read by both walks
        let source = "pub mod n {\n    pub macro m() {}\n    pub fn b() {}\n}\n";
        assert_eq!(keys(source), ["n::b"]);
        assert_eq!(left_out(source), 1);
    }

    #[test]
    fn a_cfg_that_cannot_be_read_is_a_failure_and_never_a_guess_that_it_is_not_a_test() {
        for source in [
            "#[cfg()]\npub fn f() {}\n",
            "#[cfg(1)]\npub fn f() {}\n",
            "#[cfg(all(test, 1))]\npub fn f() {}\n",
            "#[cfg(any(unix, \"x\"))]\npub fn f() {}\n",
            "pub struct S;\nimpl S {\n    #[cfg(all(test, 1))]\n    pub fn f() {}\n}\n",
            "pub mod m {\n    #[cfg(1)]\n    pub fn f() {}\n}\n",
            "pub trait Tr {\n    #[cfg(1)]\n    fn f();\n}\n",
            "#[cfg(1)]\npub fn f() -> u8;\n",
            // read although the declaration cannot be an Item anyway
            "#[cfg(1)]\nfn f() {}\n",
        ] {
            let reply = items(&Request {
                protocol: PROTOCOL,
                source: source.to_string(),
            });
            match reply {
                Err(Error::Source(message)) => {
                    assert!(message.contains("cfg"), "{source:?}: {message}");
                }
                other => panic!("{source:?}: expected a source error, got {other:?}"),
            }
        }
        // a condition that is read, in every form, is not an error
        assert_eq!(keys("#[cfg(not(test))]\npub fn f() {}\n"), ["f"]);
        assert_eq!(keys("#[cfg(all())]\npub fn f() {}\n"), ["f"]);
        assert_eq!(keys("#[cfg(feature = \"x\")]\npub fn f() {}\n"), ["f"]);
    }
}
