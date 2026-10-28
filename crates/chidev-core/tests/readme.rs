//! README's check tables are rendered from the registry (`builtin::all()`): this test fails when one is stale and
//! rewrites them under `CHIDEV_BLESS=1` (`just readme`), so the code is the one source.
//!
//! A table sits between `<!-- checks: <prefix> ... -->` and `<!-- /checks -->`, and lists the builtin checks whose id
//! starts with one of the prefixes (the text before the first `.`), in the order of the registry.

use chidev_core::domain::check::builtin;

const README: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md");
const OPEN: &str = "<!-- checks:";
const CLOSE: &str = "<!-- /checks -->";

/// The prefix of a rule id: the text before its first `.`.
fn prefix(id: &str) -> &str {
    id.split('.').next().unwrap_or(id)
}

/// The table of the builtin checks whose prefix is among `prefixes`.
fn render(prefixes: &[&str]) -> String {
    let mut out = String::from("| rule | default | moments | finds |\n|---|---|---|---|\n");
    for check in builtin::all() {
        let d = check.descriptor();
        if !prefixes.contains(&prefix(d.id.as_str())) {
            continue;
        }
        let mut level = d.default_level.word().to_string();
        if d.mandatory {
            level.push_str(", **mandatory**");
        }
        let moments: Vec<&str> = d.moments.iter().map(|m| m.word()).collect();
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            d.id,
            level,
            moments.join(", "),
            d.summary.replace('|', "\\|")
        ));
    }
    out
}

/// One block of the README: the words of its marker and the byte range of what stands between the markers.
struct Block {
    words: Vec<String>,
    body: std::ops::Range<usize>,
}

fn blocks(text: &str) -> Vec<Block> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find(OPEN) {
        let open = at + i;
        let line_end = open
            + text[open..]
                .find('\n')
                .expect("a checks marker ends its line");
        let marker = &text[open + OPEN.len()..line_end];
        let words = marker
            .trim()
            .strip_suffix("-->")
            .expect("a checks marker closes with `-->`")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let start = line_end + 1;
        let end = start
            + text[start..]
                .find(CLOSE)
                .expect("a checks block is closed by `<!-- /checks -->`");
        found.push(Block {
            words,
            body: start..end,
        });
        at = end + CLOSE.len();
    }
    found
}

#[test]
fn readme_check_tables_are_the_registry() {
    let text = std::fs::read_to_string(README).expect("README.md is readable");
    let found = blocks(&text);
    assert!(!found.is_empty(), "README.md has no checks block");

    let render_block = |b: &Block| {
        let words: Vec<&str> = b.words.iter().map(String::as_str).collect();
        render(&words)
    };

    if std::env::var_os("CHIDEV_BLESS").is_some() {
        let mut out = String::new();
        let mut from = 0;
        for b in &found {
            out.push_str(&text[from..b.body.start]);
            out.push_str(&render_block(b));
            from = b.body.end;
        }
        out.push_str(&text[from..]);
        if out != text {
            std::fs::write(README, out).expect("README.md is writable");
        }
        return;
    }

    for b in &found {
        assert_eq!(
            &text[b.body.clone()],
            render_block(b),
            "the README table for `{}` is stale: run `just readme`",
            b.words.join(" ")
        );
    }
}

#[test]
fn every_builtin_check_is_in_exactly_one_readme_table() {
    let text = std::fs::read_to_string(README).expect("README.md is readable");
    let found = blocks(&text);

    for check in builtin::all() {
        let id = check.descriptor().id.as_str();
        let n = found
            .iter()
            .filter(|b| b.words.iter().any(|w| w == prefix(id)))
            .count();
        assert_eq!(n, 1, "`{id}` is in {n} README tables, not exactly one");
    }

    for b in &found {
        for word in &b.words {
            assert!(
                builtin::all()
                    .iter()
                    .any(|c| prefix(c.descriptor().id.as_str()) == word),
                "the README table word `{word}` matches no builtin check"
            );
        }
    }
}
