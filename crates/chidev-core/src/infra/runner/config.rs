//! The `[measure]` table of `checks.toml`: what to run to build and test a project, where each Interface and Harness
//! lives in the working tree, and which command makes mutants for a language. Nothing here is guessed from the
//! project's files; a field that is not known is refused by name, so a typo does not silently measure nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::domain::vo::digest::Digest;

/// The directory under the project root a run works in, and the directories a copy never takes.
pub(super) const WORK_DIR: &str = ".chidev/measure";

/// What a copy leaves out when the file does not say: the version control data, build output, and chidev's own
/// state (which holds the runs themselves).
const DEFAULT_EXCLUDE: [&str; 3] = [".git", "target", ".chidev"];

/// The target directory a run builds into when the file sets no environment: inside the run's own directory, so
/// a mutant's artifacts never reach the project's or another run's.
const DEFAULT_ENV: [(&str, &str); 1] = [("CARGO_TARGET_DIR", "{work}/target")];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    copy_exclude: Option<Vec<String>>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    build: Vec<String>,
    test: Vec<String>,
    #[serde(default)]
    mutants: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    layout: Vec<RawLayout>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    baseline_timeout: Option<u64>,
    #[serde(default)]
    budget: Option<u64>,
    #[serde(default)]
    max_mutants: Option<usize>,
    #[serde(default)]
    seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayout {
    context: String,
    #[serde(default)]
    interface: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    path: String,
}

/// What a layout entry places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Place {
    /// An Interface's code, in this language.
    Interface {
        /// The language, which selects the provider.
        lang: String,
    },
    /// A Harness's source.
    Harness,
}

/// Where one Interface or Harness lives in the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    pub context: String,
    pub name: String,
    pub place: Place,
    /// Relative to the project root, without `..`.
    pub path: PathBuf,
}

/// The measure configuration of a project.
#[derive(Debug, Clone)]
pub(super) struct Config {
    pub copy_exclude: Vec<PathBuf>,
    /// Variable and value; the value may hold `{work}`.
    pub env: Vec<(String, String)>,
    pub build: Vec<String>,
    pub test: Vec<String>,
    /// Language to the command that makes its mutants.
    pub mutants: BTreeMap<String, Vec<String>>,
    pub layout: Vec<Layout>,
    /// For one build or one test of a mutant.
    pub timeout: Duration,
    /// For the build and the test before any change.
    pub baseline_timeout: Duration,
    /// For the whole run.
    pub budget: Duration,
    pub max_mutants: usize,
    pub seed: u64,
    /// Digest of the table as written: any change of it is a change of the settings of a run.
    pub digest: String,
}

impl Config {
    /// The place and language of the Interface `name` of `context`.
    pub(super) fn interface(&self, context: &str, name: &str) -> Option<(&Layout, &str)> {
        self.layout.iter().find_map(|l| match &l.place {
            Place::Interface { lang } if l.context == context && l.name == name => {
                Some((l, lang.as_str()))
            }
            _ => None,
        })
    }

    /// The place of the Harness `name` of `context`.
    pub(super) fn harness(&self, context: &str, name: &str) -> Option<&Layout> {
        self.layout
            .iter()
            .find(|l| l.place == Place::Harness && l.context == context && l.name == name)
    }
}

/// A command line: not empty, no NUL.
fn command(what: &str, words: &[String]) -> Result<(), String> {
    match words.first() {
        Some(program) if !program.trim().is_empty() => {}
        _ => {
            return Err(format!(
                "`{what}` is a command: a program and its arguments"
            ));
        }
    }
    if words.iter().any(|w| w.contains('\0')) {
        return Err(format!("`{what}` cannot contain NUL"));
    }
    Ok(())
}

/// A path inside the project: relative, going nowhere but down.
fn inside(what: &str, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let ok = !path.is_empty()
        && p.components().all(|c| matches!(c, Component::Normal(_)))
        && !path.contains('\0');
    if ok {
        Ok(p.to_path_buf())
    } else {
        Err(format!(
            "{what} `{path}` is a path inside the project: relative, without `..`"
        ))
    }
}

/// The configuration the table `value` (the `[measure]` of the file) states.
pub(super) fn parse(value: &toml::Value) -> Result<Config, String> {
    let raw: Raw = value
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())?;
    command("build", &raw.build)?;
    command("test", &raw.test)?;
    for (lang, words) in &raw.mutants {
        if lang.trim().is_empty() {
            return Err("a language in `mutants` has a name".to_string());
        }
        command(&format!("mutants.{lang}"), words)?;
    }
    let seconds = |what: &str, v: Option<u64>, default: u64| match v {
        Some(0) => Err(format!("`{what}` must be positive seconds")),
        Some(n) => Ok(Duration::from_secs(n)),
        None => Ok(Duration::from_secs(default)),
    };
    let timeout = seconds("timeout", raw.timeout, 30)?;
    let baseline_timeout = seconds("baseline_timeout", raw.baseline_timeout, 600)?;
    let budget = seconds("budget", raw.budget, 900)?;
    if budget.as_secs() > 3600 {
        return Err("`budget` is at most 3600 seconds".to_string());
    }
    let max_mutants = match raw.max_mutants {
        Some(0) => return Err("`max_mutants` must be at least 1".to_string()),
        Some(n) => n,
        None => 40,
    };
    let copy_exclude = match raw.copy_exclude {
        Some(list) => list
            .iter()
            .map(|p| inside("`copy_exclude` entry", p))
            .collect::<Result<Vec<_>, _>>()?,
        None => DEFAULT_EXCLUDE.iter().map(PathBuf::from).collect(),
    };
    let env = match raw.env {
        Some(env) => {
            for (k, v) in &env {
                if k.is_empty() || k.contains(['=', '\0']) || v.contains('\0') {
                    return Err(format!("`env` variable `{k}` is not a name and a value"));
                }
            }
            env.into_iter().collect()
        }
        None => DEFAULT_ENV
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let mut layout = Vec::with_capacity(raw.layout.len());
    let mut seen = BTreeSet::new();
    for l in raw.layout {
        let path = inside("`layout` path", &l.path)?;
        let (name, place) = match (l.interface, l.harness, l.lang) {
            (Some(name), None, Some(lang)) => {
                if !raw.mutants.contains_key(&lang) {
                    return Err(format!(
                        "the layout of interface `{name}` has lang `{lang}`, which `mutants` has no command for"
                    ));
                }
                (name, Place::Interface { lang })
            }
            (Some(name), None, None) => {
                return Err(format!("the layout of interface `{name}` needs a `lang`"));
            }
            (None, Some(name), None) => (name, Place::Harness),
            (None, Some(name), Some(_)) => {
                return Err(format!("the layout of harness `{name}` takes no `lang`"));
            }
            _ => {
                return Err(
                    "a layout entry places exactly one of an `interface` and a `harness`"
                        .to_string(),
                );
            }
        };
        let kind = if place == Place::Harness {
            "harness"
        } else {
            "interface"
        };
        if !seen.insert((l.context.clone(), kind, name.clone())) {
            return Err(format!(
                "the {kind} `{name}` of `{}` is laid out twice",
                l.context
            ));
        }
        layout.push(Layout {
            context: l.context,
            name,
            place,
            path,
        });
    }
    let digest = Digest::of(
        &serde_json::to_vec(value).map_err(|e| format!("cannot fingerprint the table: {e}"))?,
    )
    .to_string();
    Ok(Config {
        copy_exclude,
        env,
        build: raw.build,
        test: raw.test,
        mutants: raw.mutants,
        layout,
        timeout,
        baseline_timeout,
        budget,
        max_mutants,
        seed: raw.seed.unwrap_or(0),
        digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
build = ["cargo", "build", "--tests"]
test = ["cargo", "test"]
[mutants]
rust = ["chidev", "provider", "mutants-rust"]
[[layout]]
context = "inventory"
interface = "stock"
lang = "rust"
path = "src/stock.rs"
[[layout]]
context = "inventory"
harness = "stock-tests"
path = "tests/stock.rs"
"#;

    fn table(text: &str) -> toml::Value {
        toml::Value::Table(text.parse::<toml::Table>().unwrap())
    }

    #[test]
    fn a_minimal_table_gets_the_documented_defaults_and_finds_its_places() {
        let config = parse(&table(MINIMAL)).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.baseline_timeout, Duration::from_secs(600));
        assert_eq!(config.budget, Duration::from_secs(900));
        assert_eq!((config.max_mutants, config.seed), (40, 0));
        assert_eq!(
            config.copy_exclude,
            [".git", "target", ".chidev"].map(PathBuf::from)
        );
        assert_eq!(
            config.env,
            [("CARGO_TARGET_DIR".to_string(), "{work}/target".to_string())]
        );
        let (place, lang) = config.interface("inventory", "stock").unwrap();
        assert_eq!(
            (place.path.as_path(), lang),
            (Path::new("src/stock.rs"), "rust")
        );
        assert_eq!(
            config.harness("inventory", "stock-tests").unwrap().path,
            Path::new("tests/stock.rs")
        );
        assert!(config.interface("inventory", "stock-tests").is_none());
        assert!(config.harness("other", "stock-tests").is_none());
    }

    #[test]
    fn the_digest_follows_the_table_as_written() {
        let one = parse(&table(MINIMAL)).unwrap().digest;
        assert_eq!(one, parse(&table(MINIMAL)).unwrap().digest);
        let other = MINIMAL.replace("\"cargo\", \"test\"", "\"cargo\", \"test\", \"--quiet\"");
        assert_ne!(one, parse(&table(&other)).unwrap().digest);
    }

    #[test]
    fn what_cannot_be_measured_with_is_refused_by_name() {
        // a top-level key goes before the first table, so those edits insert ahead of `[mutants]`
        for (from, to, expected) in [
            (
                "build = [\"cargo\", \"build\", \"--tests\"]",
                "build = []",
                "build",
            ),
            ("[mutants]", "[mutantz]", "mutantz"),
            (
                "path = \"src/stock.rs\"",
                "path = \"../stock.rs\"",
                "without `..`",
            ),
            (
                "path = \"src/stock.rs\"",
                "path = \"/etc/passwd\"",
                "relative",
            ),
            ("lang = \"rust\"", "lang = \"go\"", "no command for"),
            ("lang = \"rust\"\n", "", "needs a `lang`"),
            ("[mutants]", "timeout = 0\n[mutants]", "positive"),
            ("[mutants]", "budget = 7200\n[mutants]", "3600"),
            ("[mutants]", "max_mutants = 0\n[mutants]", "at least 1"),
        ] {
            let error = parse(&table(&MINIMAL.replacen(from, to, 1))).unwrap_err();
            assert!(error.contains(expected), "{error} vs {expected}");
        }
    }

    #[test]
    fn a_place_is_laid_out_once_and_by_exactly_one_kind() {
        let twice = format!(
            "{MINIMAL}\n[[layout]]\ncontext = \"inventory\"\nharness = \"stock-tests\"\npath = \"tests/other.rs\"\n"
        );
        assert!(parse(&table(&twice)).unwrap_err().contains("twice"));
        let both = format!(
            "{MINIMAL}\n[[layout]]\ncontext = \"inventory\"\ninterface = \"a\"\nharness = \"b\"\nlang = \"rust\"\npath = \"x.rs\"\n"
        );
        assert!(parse(&table(&both)).unwrap_err().contains("exactly one"));
        let harness_lang = format!(
            "{MINIMAL}\n[[layout]]\ncontext = \"inventory\"\nharness = \"b\"\nlang = \"rust\"\npath = \"x.rs\"\n"
        );
        assert!(
            parse(&table(&harness_lang))
                .unwrap_err()
                .contains("no `lang`")
        );
    }
}
