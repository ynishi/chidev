//! The Rust mutant provider of chidev. It speaks `chidev-mutants 1`: one JSON request in, one JSON reply out, and
//! nothing else. It is a pure function of the request: the same source, `max` and `seed` give the same reply, and
//! it touches no file, no process and no clock, so a provider for another language is as small as this one and
//! chidev runs every provider the same way (as a process).
//!
//! The reply is a list of **edits**, never mutated files: each mutant gives the byte range `span` of the source,
//! the text `from` that range holds and the text `to` that replaces it. The caller checks `source[span] == from`
//! before it applies an edit, so a provider that disagrees with the text chidev holds is a protocol failure and
//! not a silent wrong mutant.
//!
//! A site is a swappable operator or boolean literal, or an integer literal (`n` becomes `n + 1`, the suffix
//! kept), or a string literal, inside a function body. Code under `#[cfg(test)]` and `#[test]` functions is not
//! mutated: it is the tests, not the thing they check. Unary minus and operators of other lengths (`->`, `=>`,
//! `+=`, `<<`) are not sites, and neither are the angle brackets of a turbofish. The operator table and the even
//! sampling of sites are ported from ryo-synth.
//!
//! # String literals (class `str`)
//!
//! A plain string literal (`"..."`) or a raw one (`r"..."`, `r#"..."#`) gives two mutants: **empty** (the text is
//! dropped) and **marker** (`*` is appended inside the quotes, so the text changes observably and stays valid).
//! A `{...}` placeholder is part of the text a format string needs, so the empty mutant keeps every placeholder
//! (`"x = {x}, {:>w$}"` becomes `"{x}{:>w$}"`) and only the text around them goes; a literal with nothing but
//! placeholders has no empty mutant, as it would be the same text. An empty literal has only the marker mutant.
//! Skipped on purpose: byte strings (`b"..."`, `br"..."`) and C strings (`c"..."`), which are not text; literals
//! inside attributes (`#[doc = "..."]`, `#[cfg(feature = "x")]`); and literals in the arguments of macros that
//! read them at compile time (`include_str!`, `include_bytes!`, `include!`, `env!`, `option_env!`, `concat!`,
//! `stringify!`, `compile_error!`, `cfg!`, `asm!`, `global_asm!`, `macro_rules!`), where a changed text is not a
//! changed message but another file, variable or no code at all.
//!
//! # Selection
//!
//! The optional request field `operators` lists the classes to make (`rel`, `eq`, `arith`, `logic`, `bool`,
//! `int`, `str`); absent, all are made. A name that is not a class is an error, and `sites` counts the sites of
//! the chosen classes only.
//!
//! A mutant's `key` is `<function>/<operator>/<n>/<line digest>`: the function (`Type::method` inside an `impl`),
//! the operator class, the position among that class's sites in the function, and four hex digits of the SHA-256
//! of the trimmed line. The two mutants of a string literal are told apart by the class in the key, `str-empty`
//! and `str-marker`; their `operator` is `str`. An edit elsewhere in the file moves none of them, so a record of
//! a mutant outlives unrelated changes.

#![warn(missing_docs)]

use std::collections::BTreeMap;

use proc_macro2::{Spacing, TokenStream, TokenTree};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use syn::visit::Visit;
use thiserror::Error;

/// The protocol version this provider speaks.
pub const PROTOCOL: u32 = 1;

/// The operators a mutant swaps (composed from joint punctuation) and the boolean literals, with the class that
/// names them in a key.
const SWAPS: [(&str, &str, &str); 12] = [
    ("<", "<=", "rel"),
    ("<=", "<", "rel"),
    (">", ">=", "rel"),
    (">=", ">", "rel"),
    ("==", "!=", "eq"),
    ("!=", "==", "eq"),
    ("+", "-", "arith"),
    ("-", "+", "arith"),
    ("&&", "||", "logic"),
    ("||", "&&", "logic"),
    ("true", "false", "bool"),
    ("false", "true", "bool"),
];

/// The class of the integer literal sites.
const INT_CLASS: &str = "int";

/// The class of the string literal sites.
const STR_CLASS: &str = "str";

/// Every class a request can select.
pub const OPERATORS: [&str; 7] = ["rel", "eq", "arith", "logic", "bool", "int", "str"];

/// What is appended to a string literal by the marker mutant.
const MARKER: &str = "*";

/// Macros whose string arguments are read at compile time as something other than a message.
const COMPILE_TIME_MACROS: [&str; 12] = [
    "macro_rules",
    "include_str",
    "include_bytes",
    "include",
    "env",
    "option_env",
    "concat",
    "stringify",
    "compile_error",
    "cfg",
    "asm",
    "global_asm",
];

/// What chidev asks for: the mutants of `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The protocol version; this provider answers only [`PROTOCOL`].
    pub protocol: u32,
    /// Where the source lives in the project. Informational: the provider reads no file.
    pub path: String,
    /// The text to mutate.
    pub source: String,
    /// The most mutants to return, at least 1. Sites beyond it are sampled evenly.
    pub max: usize,
    /// Shifts where the even sampling starts; 0 samples from the first site.
    #[serde(default)]
    pub seed: u64,
    /// The operator classes to make (see [`OPERATORS`]); absent, every class. Optional, so a request of the
    /// first protocol version without it still parses and means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operators: Option<Vec<String>>,
}

/// One single-site edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutant {
    /// A stable id: function, operator class, position in the function, digest of the line.
    pub key: String,
    /// The 1-based line of the site.
    pub line: usize,
    /// The 1-based column of the site, in characters.
    pub col: usize,
    /// The operator class: `rel`, `eq`, `arith`, `logic`, `bool`, `int` or `str`.
    pub operator: String,
    /// The byte range `[start, end)` of the source the edit replaces.
    pub span: [usize; 2],
    /// What the source holds in `span`.
    pub from: String,
    /// What replaces it.
    pub to: String,
}

/// What the provider answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// The protocol version.
    pub protocol: u32,
    /// How many sites the source has, before sampling.
    pub sites: usize,
    /// The chosen mutants, in the order of the source.
    pub mutants: Vec<Mutant>,
}

/// Why no reply could be made.
#[derive(Debug, Error)]
pub enum Error {
    /// The request names a protocol version this provider does not speak.
    #[error("protocol {0} is not spoken here; this provider speaks {PROTOCOL}")]
    Protocol(u32),
    /// The request is not a request.
    #[error("not a chidev-mutants request: {0}")]
    Request(serde_json::Error),
    /// The request asks for no mutant.
    #[error("`max` must be at least 1")]
    NoMax,
    /// The request selects a class this provider does not make.
    #[error("`{0}` is not an operator class of this provider (it makes {OPERATORS:?})")]
    Operator(String),
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
    let reply = mutants(&request)?;
    serde_json::to_vec(&reply).map_err(Error::Reply)
}

/// The mutants of the request's source.
pub fn mutants(request: &Request) -> Result<Reply, Error> {
    if request.protocol != PROTOCOL {
        return Err(Error::Protocol(request.protocol));
    }
    if request.max == 0 {
        return Err(Error::NoMax);
    }
    if let Some(chosen) = &request.operators
        && let Some(bad) = chosen.iter().find(|o| !OPERATORS.contains(&o.as_str()))
    {
        return Err(Error::Operator(bad.clone()));
    }
    let wanted = |class: &str| {
        request
            .operators
            .as_ref()
            .is_none_or(|chosen| chosen.iter().any(|o| o == class))
    };
    let source = request.source.as_str();
    let file = syn::parse_file(source).map_err(|e| Error::Source(e.to_string()))?;
    let stream: TokenStream = source
        .parse()
        .map_err(|e: proc_macro2::LexError| Error::Source(e.to_string()))?;
    let mut finder = Bodies::default();
    finder.visit_file(&file);
    let mut sites = Vec::new();
    scan(stream, &finder.bodies, &mut sites, false);
    sites.retain(|s| wanted(s.class));
    sites.sort_by_key(|s| (s.start, s.end));
    let total = sites.len();
    let mut counts: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let mut keyed = Vec::with_capacity(total);
    for site in sites {
        let name = innermost(&finder.bodies, &site).unwrap_or_default();
        let key_class = site.key_class.unwrap_or(site.class);
        let n = counts.entry((name.clone(), key_class)).or_insert(0);
        *n += 1;
        let line = lines.get(site.line - 1).map_or("", |l| l.trim());
        let digest = Sha256::digest(line.as_bytes());
        let key = format!("{name}/{key_class}/{n}/{:02x}{:02x}", digest[0], digest[1]);
        keyed.push(Mutant {
            key,
            line: site.line,
            col: site.column + 1,
            operator: site.class.to_string(),
            span: [site.start, site.end],
            from: site.from,
            to: site.to,
        });
    }
    Ok(Reply {
        protocol: PROTOCOL,
        sites: total,
        mutants: sample(keyed, request.max, request.seed),
    })
}

/// Up to `max` of `all`, spread evenly; `seed` moves the first pick within the first step.
fn sample(all: Vec<Mutant>, max: usize, seed: u64) -> Vec<Mutant> {
    let step = f64::max(1.0, all.len() as f64 / max as f64);
    let mut at = (seed as f64) % step;
    let mut chosen = Vec::new();
    let mut all: Vec<Option<Mutant>> = all.into_iter().map(Some).collect();
    while (at as usize) < all.len() && chosen.len() < max {
        if let Some(m) = all[at as usize].take() {
            chosen.push(m);
        }
        at += step;
    }
    chosen.sort_by_key(|m| m.span[0]);
    chosen
}

/// A place a mutant can be made, before it is named.
#[derive(Debug)]
struct Site {
    start: usize,
    end: usize,
    line: usize,
    column: usize,
    /// The operator class of the reply.
    class: &'static str,
    /// The class a key names, when it tells apart several mutants of one site.
    key_class: Option<&'static str>,
    from: String,
    to: String,
}

/// A function body: where its braces are, and what it is called.
#[derive(Debug)]
struct Body {
    name: String,
    /// The byte after the opening brace.
    inside: usize,
    /// The byte of the closing brace.
    close: usize,
}

/// The innermost function body a site is in.
fn innermost(bodies: &[Body], site: &Site) -> Option<String> {
    bodies
        .iter()
        .filter(|b| b.inside <= site.start && site.end <= b.close)
        .min_by_key(|b| b.close - b.inside)
        .map(|b| b.name.clone())
}

/// Collects the bodies of the functions that are not tests.
#[derive(Default)]
struct Bodies {
    bodies: Vec<Body>,
    owner: Vec<String>,
}

impl Bodies {
    fn push(&mut self, name: String, block: &syn::Block) {
        self.bodies.push(Body {
            name,
            inside: block.brace_token.span.open().byte_range().end,
            close: block.brace_token.span.close().byte_range().start,
        });
    }

    fn named(&self, ident: &syn::Ident) -> String {
        match self.owner.last() {
            Some(owner) if !owner.is_empty() => format!("{owner}::{ident}"),
            _ => ident.to_string(),
        }
    }
}

/// Whether the attributes make the item test code: `#[test]` or `#[cfg(test)]`.
fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && a.parse_args::<syn::Meta>()
                    .is_ok_and(|m| matches!(m, syn::Meta::Path(p) if p.is_ident("test"))))
    })
}

impl<'a> Visit<'a> for Bodies {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if is_test(&f.attrs) {
            return;
        }
        let name = self.named(&f.sig.ident);
        self.push(name, &f.block);
        syn::visit::visit_item_fn(self, f);
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        if is_test(&i.attrs) {
            return;
        }
        let ty = match &*i.self_ty {
            syn::Type::Path(p) => p
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        };
        self.owner.push(ty);
        syn::visit::visit_item_impl(self, i);
        self.owner.pop();
    }

    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        if is_test(&f.attrs) {
            return;
        }
        let name = self.named(&f.sig.ident);
        self.push(name, &f.block);
        syn::visit::visit_impl_item_fn(self, f);
    }

    fn visit_trait_item_fn(&mut self, f: &'a syn::TraitItemFn) {
        if let Some(block) = &f.default {
            self.push(f.sig.ident.to_string(), block);
        }
        syn::visit::visit_trait_item_fn(self, f);
    }
}

/// Finds the sites of one token stream, and of the groups inside it. An operator is binary only after an operand
/// of the same stream, so the first `-` of a group is unary and is not a site.
///
/// `quiet` is set inside an attribute or a macro that reads its strings at compile time: no string site there.
fn scan(stream: TokenStream, bodies: &[Body], sites: &mut Vec<Site>, quiet: bool) {
    let flat: Vec<TokenTree> = stream.into_iter().collect();
    // The depth of turbofish generic arguments (`::<` ... `>`): their angle brackets are not comparisons.
    let mut turbofish = 0usize;
    let mut i = 0;
    while i < flat.len() {
        match &flat[i] {
            TokenTree::Group(g) => {
                let hidden = quiet || is_attribute(&flat, i, g) || is_compile_time(&flat, i);
                scan(g.stream(), bodies, sites, hidden);
            }
            TokenTree::Punct(p) => {
                let mut op = p.as_char().to_string();
                let mut j = i;
                while let TokenTree::Punct(q) = &flat[j] {
                    if q.spacing() != Spacing::Joint || j + 1 >= flat.len() {
                        break;
                    }
                    match &flat[j + 1] {
                        TokenTree::Punct(next) => {
                            op.push(next.as_char());
                            j += 1;
                        }
                        _ => break,
                    }
                }
                let after_operand = i > 0 && !matches!(&flat[i - 1], TokenTree::Punct(_));
                if op == "::<"
                    || (op == "<"
                        && i > 0
                        && matches!(&flat[i - 1], TokenTree::Punct(q) if q.as_char() == ':'))
                {
                    turbofish += 1;
                } else if op == ">" && turbofish > 0 {
                    turbofish -= 1;
                } else if let Some((_, to, class)) = SWAPS.iter().find(|(o, _, _)| *o == op)
                    && (!matches!(op.as_str(), "-" | "<" | ">") || after_operand)
                {
                    let start = p.span().byte_range().start;
                    let at = p.span().start();
                    push(sites, bodies, start, &op, to, class, at);
                }
                i = j + 1;
                continue;
            }
            TokenTree::Ident(id) => {
                let name = id.to_string();
                if let Some((_, to, class)) = SWAPS.iter().find(|(o, _, _)| *o == name) {
                    push(
                        sites,
                        bodies,
                        id.span().byte_range().start,
                        &name,
                        to,
                        class,
                        id.span().start(),
                    );
                }
            }
            TokenTree::Literal(l) => {
                let text = l.to_string();
                if !quiet {
                    push_strings(sites, bodies, l, &text);
                }
                let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
                let rest = &text[digits.len()..];
                let suffix_ok = rest.is_empty()
                    || (rest.chars().all(|c| c.is_ascii_alphanumeric())
                        && rest.starts_with(['i', 'u']));
                if !digits.is_empty()
                    && suffix_ok
                    && let Ok(v) = digits.parse::<u128>()
                    && let Some(next) = v.checked_add(1)
                {
                    push(
                        sites,
                        bodies,
                        l.span().byte_range().start,
                        &digits,
                        &next.to_string(),
                        INT_CLASS,
                        l.span().start(),
                    );
                }
            }
        }
        i += 1;
    }
}

/// Whether the group at `i` is the `[...]` of an attribute (`#[...]`, `#![...]`).
fn is_attribute(flat: &[TokenTree], i: usize, g: &proc_macro2::Group) -> bool {
    let punct = |at: Option<usize>, c: char| {
        at.and_then(|k| flat.get(k))
            .is_some_and(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == c))
    };
    g.delimiter() == proc_macro2::Delimiter::Bracket
        && (punct(i.checked_sub(1), '#')
            || (punct(i.checked_sub(1), '!') && punct(i.checked_sub(2), '#')))
}

/// Whether the group at `i` is the argument of a macro in [`COMPILE_TIME_MACROS`] (`name!(...)`, or the body of
/// `macro_rules! name { ... }`).
fn is_compile_time(flat: &[TokenTree], i: usize) -> bool {
    let named = |at: Option<usize>| {
        at.and_then(|k| flat.get(k)).is_some_and(
            |t| matches!(t, TokenTree::Ident(id) if COMPILE_TIME_MACROS.contains(&id.to_string().as_str())),
        )
    };
    let bang = |at: Option<usize>| {
        at.and_then(|k| flat.get(k))
            .is_some_and(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '!'))
    };
    (bang(i.checked_sub(1)) && named(i.checked_sub(2)))
        || (matches!(
            i.checked_sub(1).and_then(|k| flat.get(k)),
            Some(TokenTree::Ident(_))
        ) && bang(i.checked_sub(2))
            && named(i.checked_sub(3)))
}

/// The parts of a text string literal: what opens it (`"`, `r"`, `r#"`), its text, and what closes it. `None` for
/// every other literal, byte and C strings included.
fn string_parts(text: &str) -> Option<(&str, &str, &str)> {
    let after_r = text.strip_prefix('r');
    let hashes = after_r.map_or(0, |rest| rest.chars().take_while(|c| *c == '#').count());
    let open = match after_r {
        Some(_) => 1 + hashes + 1,
        None => 1,
    };
    let opener = text.get(..open)?;
    if !opener.ends_with('"') {
        return None;
    }
    let close_len = 1 + hashes;
    let body_end = text.len().checked_sub(close_len)?;
    if body_end < open {
        return None;
    }
    let closer = text.get(body_end..)?;
    if !closer.starts_with('"') || closer[1..].chars().any(|c| c != '#') {
        return None;
    }
    Some((opener, text.get(open..body_end)?, closer))
}

/// `text` with only its `{...}` placeholders left; `{{` and `}}` are escapes and go with the text around.
fn placeholders(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(['{', '}']) {
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rest = &tail[2..];
        } else if tail.starts_with('{')
            && let Some(close) = tail.find('}')
        {
            kept.push_str(&tail[..=close]);
            rest = &tail[close + 1..];
        } else {
            rest = &tail[1..];
        }
    }
    kept
}

/// Records the two mutants of a string literal (see the crate documentation), if it is inside a function body.
fn push_strings(sites: &mut Vec<Site>, bodies: &[Body], l: &proc_macro2::Literal, text: &str) {
    let Some((open, body, close)) = string_parts(text) else {
        return;
    };
    let start = l.span().byte_range().start;
    let end = start + text.len();
    if !bodies.iter().any(|b| b.inside <= start && end <= b.close) {
        return;
    }
    let at = l.span().start();
    let kept = placeholders(body);
    let emptied = format!("{open}{kept}{close}");
    let marked = format!("{open}{body}{MARKER}{close}");
    for (key_class, to) in [("str-empty", emptied), ("str-marker", marked)] {
        if to != text {
            sites.push(Site {
                start,
                end,
                line: at.line,
                column: at.column,
                class: STR_CLASS,
                key_class: Some(key_class),
                from: text.to_string(),
                to,
            });
        }
    }
}

/// Records a site if it is inside a function body.
fn push(
    sites: &mut Vec<Site>,
    bodies: &[Body],
    start: usize,
    from: &str,
    to: &str,
    class: &'static str,
    at: proc_macro2::LineColumn,
) {
    let end = start + from.len();
    if bodies.iter().any(|b| b.inside <= start && end <= b.close) {
        sites.push(Site {
            start,
            end,
            line: at.line,
            column: at.column,
            class,
            key_class: None,
            from: from.to_string(),
            to: to.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(source: &str, max: usize, seed: u64) -> Request {
        Request {
            protocol: PROTOCOL,
            path: "src/lib.rs".into(),
            source: source.into(),
            max,
            seed,
            operators: None,
        }
    }

    fn only(source: &str, operators: &[&str]) -> Result<Reply, Error> {
        mutants(&Request {
            operators: Some(operators.iter().map(ToString::to_string).collect()),
            ..request(source, 100, 0)
        })
    }

    fn all(source: &str) -> Reply {
        mutants(&request(source, 100, 0)).expect("a reply")
    }

    fn edits(reply: &Reply) -> Vec<String> {
        reply
            .mutants
            .iter()
            .map(|m| format!("{} -> {}", m.from, m.to))
            .collect()
    }

    const SOURCE: &str = "pub fn f(a: i64, v: &[u8]) -> bool {\n    let n = -1 + a;\n    let _ = \"7\".parse::<i64>();\n    v.len() > 2 && n == 0 || true\n}\n";

    #[test]
    fn the_operator_table_gives_every_site_of_a_body_and_none_of_a_signature() {
        let reply = all(SOURCE);
        // not sites: `->` in the signature, the unary `-`, the turbofish `::<i64>`
        assert_eq!(
            edits(&reply),
            [
                "1 -> 2",
                "+ -> -",
                "\"7\" -> \"\"",
                "\"7\" -> \"7*\"",
                "> -> >=",
                "2 -> 3",
                "&& -> ||",
                "== -> !=",
                "0 -> 1",
                "|| -> &&",
                "true -> false"
            ]
        );
        assert_eq!(reply.sites, 11);
        let classes: Vec<&str> = reply.mutants.iter().map(|m| m.operator.as_str()).collect();
        assert_eq!(
            classes,
            [
                "int", "arith", "str", "str", "rel", "int", "logic", "eq", "int", "logic", "bool"
            ]
        );
    }

    #[test]
    fn every_edit_is_exactly_what_the_source_holds_in_its_span() {
        let reply = all(SOURCE);
        for m in &reply.mutants {
            assert_eq!(&SOURCE[m.span[0]..m.span[1]], m.from, "{}", m.key);
        }
        // and the position is where a person would look
        let gt = &reply.mutants[4];
        assert_eq!((gt.line, gt.col), (4, 13));
        assert_eq!(
            SOURCE.split('\n').nth(3).and_then(|l| l.get(gt.col - 1..)),
            Some("> 2 && n == 0 || true")
        );
    }

    #[test]
    fn a_suffix_stays_and_a_multibyte_text_moves_no_span() {
        let source = "fn f() -> u64 {\n    let s = \"héllo\";\n    7u64 + s.len() as u64\n}\n";
        let reply = all(source);
        for m in &reply.mutants {
            assert_eq!(&source[m.span[0]..m.span[1]], m.from);
        }
        assert_eq!(
            edits(&reply),
            [
                "\"héllo\" -> \"\"",
                "\"héllo\" -> \"héllo*\"",
                "7 -> 8",
                "+ -> -"
            ]
        );
        let mut edited = source.to_string();
        edited.replace_range(reply.mutants[2].span[0]..reply.mutants[2].span[1], "8");
        assert!(edited.contains("8u64 + s.len()"));
    }

    #[test]
    fn the_same_request_gives_the_same_reply_byte_for_byte() {
        let one = handle(
            serde_json::to_vec(&request(SOURCE, 4, 3))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let two = handle(
            serde_json::to_vec(&request(SOURCE, 4, 3))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(one, two);
    }

    #[test]
    fn sampling_is_even_and_the_seed_moves_it() {
        let source =
            "fn f(a: i32) -> bool {\n    a > 1 && a > 2 && a > 3 && a > 4 && a > 5 && a > 6\n}\n";
        assert_eq!(all(source).sites, 17);
        let from_start = mutants(&request(source, 4, 0)).unwrap();
        assert_eq!(from_start.mutants.len(), 4);
        assert_eq!(from_start.sites, 17);
        // 17 sites over 4 mutants: every 4.25th, from the first
        let at = |r: &Reply| r.mutants.iter().map(|m| m.span[0]).collect::<Vec<_>>();
        let shifted = mutants(&request(source, 4, 1)).unwrap();
        assert_ne!(at(&from_start), at(&shifted));
        assert_eq!(at(&shifted), at(&mutants(&request(source, 4, 1)).unwrap()));
        // asking for more than there are gives them all
        assert_eq!(mutants(&request(source, 50, 9)).unwrap().mutants.len(), 17);
    }

    #[test]
    fn test_code_and_code_outside_a_body_are_not_mutated() {
        let source = "const LIMIT: u32 = 10;\nstruct S { n: u32 }\nfn real(a: u32) -> bool { a < 3 }\n#[test]\nfn a_test() { assert!(1 + 1 == 2); }\n#[cfg(test)]\nmod tests {\n    fn helper() -> bool { 4 > 3 }\n}\n";
        let reply = all(source);
        assert_eq!(edits(&reply), ["< -> <=", "3 -> 4"]);
    }

    #[test]
    fn methods_are_named_with_their_type_and_a_key_survives_an_edit_elsewhere() {
        let before =
            "struct S;\nimpl S {\n    fn fits(&self, a: u32) -> bool {\n        a <= 5\n    }\n}\n";
        let after = format!("// a new first line\nfn other() -> u32 {{\n    1\n}}\n{before}");
        let key_of = |source: &str| {
            let reply = all(source);
            reply
                .mutants
                .iter()
                .find(|m| m.operator == "rel")
                .map(|m| m.key.clone())
                .expect("a rel mutant")
        };
        let key = key_of(before);
        assert!(key.starts_with("S::fits/rel/1/"), "{key}");
        assert_eq!(key, key_of(&after));
        // the same operator on a changed line is another key
        let moved = before.replace("a <= 5", "a <= 6");
        assert_ne!(key, key_of(&moved));
    }

    #[test]
    fn keys_of_one_reply_are_distinct() {
        let reply = all(SOURCE);
        let mut keys: Vec<&str> = reply.mutants.iter().map(|m| m.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), reply.mutants.len());
    }

    fn strs(source: &str) -> Vec<(String, String, String)> {
        only(source, &["str"])
            .expect("a reply")
            .mutants
            .into_iter()
            .map(|m| (m.key, m.from, m.to))
            .collect()
    }

    #[test]
    fn a_string_gets_an_empty_and_a_marker_mutant_that_name_themselves_apart() {
        let source = "fn f() -> String {\n    String::from(\"no stock\")\n}\n";
        let reply = only(source, &["str"]).unwrap();
        assert_eq!(reply.sites, 2);
        assert_eq!(
            edits(&reply),
            ["\"no stock\" -> \"\"", "\"no stock\" -> \"no stock*\""]
        );
        assert!(reply.mutants.iter().all(|m| m.operator == "str"));
        assert!(reply.mutants[0].key.starts_with("f/str-empty/1/"));
        assert!(reply.mutants[1].key.starts_with("f/str-marker/1/"));
        for m in &reply.mutants {
            assert_eq!(&source[m.span[0]..m.span[1]], m.from);
        }
        // an empty string has only the marker; it cannot be emptied further
        let blank = strs("fn f() -> &'static str { \"\" }");
        assert_eq!(blank.len(), 1);
        assert_eq!(blank[0].2, "\"*\"");
    }

    #[test]
    fn a_format_string_keeps_every_placeholder_in_both_mutants() {
        let source = r#"fn f(n: u32, w: usize, name: &str) -> String {
    format!("{name}: {n:>w$} of {{braces}} {0:?}", n)
}
"#;
        let edits = strs(source);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].2, r#""{name}{n:>w$}{0:?}""#);
        assert_eq!(edits[1].2, r#""{name}: {n:>w$} of {{braces}} {0:?}*""#);
        // nothing but a placeholder: the emptied text would be the same, so there is the marker only
        let bare = strs(r#"fn f(n: u32) -> String { format!("{n}") }"#);
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].2, r#""{n}*""#);
    }

    #[test]
    fn a_raw_string_keeps_its_delimiters_and_byte_strings_are_not_text() {
        let raw = strs("fn f() -> &'static str { r#\"say \"hi\"\"# }");
        assert_eq!(raw[0].2, "r#\"\"#");
        assert_eq!(raw[1].2, "r#\"say \"hi\"*\"#");
        let source = "fn f() -> usize { b\"abc\".len() + br\"x\".len() + c\"y\".to_bytes().len() }";
        assert!(strs(source).is_empty());
        // a char is not a string
        assert!(strs("fn f() -> char { 'a' }").is_empty());
    }

    #[test]
    fn strings_in_attributes_test_code_and_compile_time_macros_are_not_sites() {
        let source = r##"
#[doc = "outside"]
fn f() -> String {
    #[allow(clippy::all, reason = "inner")]
    let a = include_str!("data.txt");
    let b = concat!("a", "b");
    let c = env!("HOME");
    /// a doc comment on a statement
    let d = "real";
    let _ = (a, b, c);
    d.to_string()
}
#[test]
fn a_test() { assert_eq!(f(), "real"); }
#[cfg(test)]
mod tests { fn helper() -> &'static str { "helper" } }
macro_rules! m { () => { "in a macro definition" }; }
"##;
        let found = strs(source);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|(_, from, _)| from == "\"real\""));
    }

    #[test]
    fn a_string_mutant_applied_still_parses_and_the_key_survives_an_edit_elsewhere() {
        let before = "fn msg(n: u32) -> String {\n    format!(\"{n} left\")\n}\n";
        let after = format!("// new\nfn other() -> u32 {{ 1 }}\n{before}");
        let keys = |s: &str| strs(s).into_iter().map(|(k, _, _)| k).collect::<Vec<_>>();
        assert_eq!(keys(before), keys(&after));
        for m in only(before, &["str"]).unwrap().mutants {
            let mut edited = before.to_string();
            edited.replace_range(m.span[0]..m.span[1], &m.to);
            syn::parse_file(&edited).expect("the mutant parses");
        }
    }

    #[test]
    fn operators_select_the_classes_and_the_default_is_all() {
        let source = "fn f(a: u32) -> String {\n    if a > 1 { \"big\".into() } else { \"small\".into() }\n}\n";
        let everything = all(source);
        let classes = |r: &Reply| {
            let mut c: Vec<String> = r.mutants.iter().map(|m| m.operator.clone()).collect();
            c.dedup();
            c
        };
        assert_eq!(classes(&everything), ["rel", "int", "str"]);
        assert_eq!(only(source, &OPERATORS).unwrap().sites, everything.sites);
        let no_str = only(source, &["rel", "eq", "arith", "logic", "bool", "int"]).unwrap();
        assert_eq!(classes(&no_str), ["rel", "int"]);
        assert_eq!(no_str.sites, everything.sites - 4);
        let only_str = only(source, &["str"]).unwrap();
        assert_eq!((only_str.sites, only_str.mutants.len()), (4, 4));
        assert_eq!(only(source, &["rel"]).unwrap().mutants.len(), 1);
        // a name that is no class is refused by name, and the old request (no field) parses
        assert!(matches!(only(source, &["strings"]), Err(Error::Operator(o)) if o == "strings"));
        let old = handle(
            br#"{"protocol":1,"path":"a.rs","source":"fn f() -> &'static str { \"x\" }","max":9}"#,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&old).unwrap();
        assert_eq!(value["sites"], 2);
        let narrowed = handle(br#"{"protocol":1,"path":"a.rs","source":"fn f() -> &'static str { \"x\" }","max":9,"operators":["rel"]}"#)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&narrowed).unwrap();
        assert_eq!(value["sites"], 0);
    }

    #[test]
    fn string_mutants_are_deterministic() {
        let source = "fn f() -> String { format!(\"{}-{}\", \"a\", \"b\") }";
        let bytes = |seed| handle(&serde_json::to_vec(&request(source, 3, seed)).unwrap()).unwrap();
        assert_eq!(bytes(2), bytes(2));
        assert_eq!(strs(source), strs(source));
    }

    #[test]
    fn a_bad_request_is_refused_by_name() {
        assert!(matches!(
            mutants(&Request {
                protocol: 2,
                ..request("fn f() {}", 1, 0)
            }),
            Err(Error::Protocol(2))
        ));
        assert!(matches!(
            mutants(&request("fn f() {}", 0, 0)),
            Err(Error::NoMax)
        ));
        assert!(matches!(
            mutants(&request("fn (", 1, 0)),
            Err(Error::Source(_))
        ));
        assert!(matches!(handle(b"[]"), Err(Error::Request(_))));
        // a source with no site is an empty reply, not an error
        let none = all("fn f() {}");
        assert_eq!((none.sites, none.mutants.len()), (0, 0));
    }

    #[test]
    fn the_reply_is_the_json_of_the_protocol() {
        let bytes = handle(
            br#"{"protocol":1,"path":"src/a.rs","source":"fn f(a: u8) -> bool { a > 1 }","max":40,"seed":0}"#,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["protocol"], 1);
        assert_eq!(value["sites"], 2);
        let first = &value["mutants"][0];
        assert_eq!(first["operator"], "rel");
        assert_eq!(first["from"], ">");
        assert_eq!(first["to"], ">=");
        assert_eq!(first["span"], serde_json::json!([24, 25]));
        assert_eq!(first["line"], 1);
        assert_eq!(first["col"], 25);
    }
}
