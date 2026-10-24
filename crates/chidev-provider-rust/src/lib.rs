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
//! kept), inside a function body. Code under `#[cfg(test)]` and `#[test]` functions is not mutated: it is the
//! tests, not the thing they check. Unary minus and operators of other lengths (`->`, `=>`, `+=`, `<<`) are not
//! sites, and neither are the angle brackets of a turbofish. The operator table and the even sampling of sites are
//! ported from ryo-synth.
//!
//! A mutant's `key` is `<function>/<operator>/<n>/<line digest>`: the function (`Type::method` inside an `impl`),
//! the operator class, the position among that class's sites in the function, and four hex digits of the SHA-256
//! of the trimmed line. An edit elsewhere in the file moves none of them, so a record of a mutant outlives
//! unrelated changes.

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
    /// The operator class: `rel`, `eq`, `arith`, `logic`, `bool` or `int`.
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
    let source = request.source.as_str();
    let file = syn::parse_file(source).map_err(|e| Error::Source(e.to_string()))?;
    let stream: TokenStream = source
        .parse()
        .map_err(|e: proc_macro2::LexError| Error::Source(e.to_string()))?;
    let mut finder = Bodies::default();
    finder.visit_file(&file);
    let mut sites = Vec::new();
    scan(stream, &finder.bodies, &mut sites);
    sites.sort_by_key(|s| (s.start, s.end));
    let total = sites.len();
    let mut counts: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let mut keyed = Vec::with_capacity(total);
    for site in sites {
        let name = innermost(&finder.bodies, &site).unwrap_or_default();
        let n = counts.entry((name.clone(), site.class)).or_insert(0);
        *n += 1;
        let line = lines.get(site.line - 1).map_or("", |l| l.trim());
        let digest = Sha256::digest(line.as_bytes());
        let key = format!(
            "{name}/{}/{n}/{:02x}{:02x}",
            site.class, digest[0], digest[1]
        );
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
    class: &'static str,
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
fn scan(stream: TokenStream, bodies: &[Body], sites: &mut Vec<Site>) {
    let flat: Vec<TokenTree> = stream.into_iter().collect();
    // The depth of turbofish generic arguments (`::<` ... `>`): their angle brackets are not comparisons.
    let mut turbofish = 0usize;
    let mut i = 0;
    while i < flat.len() {
        match &flat[i] {
            TokenTree::Group(g) => scan(g.stream(), bodies, sites),
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
        }
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
                "> -> >=",
                "2 -> 3",
                "&& -> ||",
                "== -> !=",
                "0 -> 1",
                "|| -> &&",
                "true -> false"
            ]
        );
        assert_eq!(reply.sites, 9);
        let classes: Vec<&str> = reply.mutants.iter().map(|m| m.operator.as_str()).collect();
        assert_eq!(
            classes,
            [
                "int", "arith", "rel", "int", "logic", "eq", "int", "logic", "bool"
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
        let gt = &reply.mutants[2];
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
        assert_eq!(edits(&reply), ["7 -> 8", "+ -> -"]);
        let mut edited = source.to_string();
        edited.replace_range(reply.mutants[0].span[0]..reply.mutants[0].span[1], "8");
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
