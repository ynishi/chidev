//! The `[measure]` table of `checks.toml`: what to run to build and test a project, where each Interface and Harness
//! lives in the working tree, which command makes mutants for a language (`[measure.mutants]`), which command finds
//! the Items of an Interface from its code (`[measure.items]`), and (`[measure.sim.<context>]`) how to run the sim
//! script of a Context against its program. Nothing here is guessed from the project's files; a field
//! that is not known is refused by name, so a typo does not silently measure nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::domain::vo::digest::Digest;
use crate::domain::vo::measure::Operator;

/// The directory under the project root a run works in, and the directories a copy never takes.
pub(super) const WORK_DIR: &str = ".chidev/measure";

/// The most seeds and the most second runs a simulation may be told to make.
const MAX_SEEDS: u32 = 10_000;
const MAX_SAMPLE: usize = 1_000;

/// What a copy leaves out when the file does not say: the version control data, build output, and chidev's own
/// state (which holds the runs themselves).
const DEFAULT_EXCLUDE: [&str; 3] = [".git", "target", ".chidev"];

/// The placeholder for the directory of a run.
const RUN: &str = "{run}";
/// The placeholder for the root of the copy a run works in.
const TREE: &str = "{tree}";
/// The placeholder that named one of the two places before each had a name of its own. It is refused everywhere,
/// so a configuration that still has it does not run with the text left as it is.
const RETIRED: &str = "{work}";

/// The target directory a run builds into when the file sets no environment: inside the run's own directory, so
/// a mutant's artifacts never reach the project's or another run's.
const DEFAULT_ENV: [(&str, &str); 1] = [("CARGO_TARGET_DIR", "{run}/target")];

/// A value of the configuration that may name the places of a run: the value of an `env` variable and a word of
/// the `program` of a sim. `{run}` is the run's directory and `{tree}` the root of the copy the run works in (the
/// second copy of a sim, which its second runs are made in, is `rerun-tree` in the same directory, so there `{tree}`
/// is its root); any other `{...}` stays as it is. Only [`RunDirs::expand`] reads the text, so the fields that are
/// a `Template` are the ones that are expanded, and [`check_placeholders`] refuses a placeholder anywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Template(String);

/// The two places of one run: its directory, and the root of the copy of the project under it (`tree`, or
/// `rerun-tree` for the second copy of a sim). The names `{run}` and `{tree}` of a [`Template`] stand for them.
#[derive(Debug, Clone)]
pub(super) struct RunDirs {
    /// The directory of the run, where its output and its target directory go.
    pub dir: PathBuf,
    /// The root of the copy the run works in.
    pub tree: PathBuf,
}

impl RunDirs {
    /// The first copy of the run in `dir`.
    pub(super) fn of(dir: PathBuf) -> Self {
        Self {
            tree: dir.join("tree"),
            dir,
        }
    }

    /// The same run with its second, fresh copy: `{tree}` is `rerun-tree` there.
    pub(super) fn rerun(&self) -> Self {
        Self {
            dir: self.dir.clone(),
            tree: self.dir.join("rerun-tree"),
        }
    }

    /// `template` with `{run}` made the directory of the run and `{tree}` the root of the copy. Any other `{...}`
    /// is left as it is: a script may need a brace in an argument.
    pub(super) fn expand(&self, template: &Template) -> String {
        let (dir, tree) = (self.dir.to_string_lossy(), self.tree.to_string_lossy());
        let mut out = String::with_capacity(template.0.len());
        let mut rest = template.0.as_str();
        while let Some(at) = rest.find('{') {
            out.push_str(&rest[..at]);
            let tail = &rest[at..];
            if let Some(after) = tail.strip_prefix(RUN) {
                out.push_str(&dir);
                rest = after;
            } else if let Some(after) = tail.strip_prefix(TREE) {
                out.push_str(&tree);
                rest = after;
            } else {
                out.push('{');
                rest = &tail[1..];
            }
        }
        out.push_str(rest);
        out
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    copy_exclude: Option<Vec<String>>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    build: Vec<String>,
    #[serde(default)]
    test: Option<Vec<String>>,
    #[serde(default)]
    mutants: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    items: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    sim: BTreeMap<String, RawSim>,
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
    #[serde(default)]
    operators: Option<Vec<String>>,
}

/// `[measure.sim.<context>]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSim {
    interpreter: Vec<String>,
    program: Vec<String>,
    #[serde(default)]
    seeds: Option<u32>,
    #[serde(default)]
    sample: Option<usize>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    budget: Option<u64>,
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
    /// An Interface's code. The language selects the provider that makes its mutants and the one that finds its
    /// Items; an Interface that is only simulated, and whose Items are written, has none.
    Interface {
        /// The language, when the Interface is mutated or its Items are found from the code.
        lang: Option<String>,
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

/// How the sim script of one Context is run.
#[derive(Debug, Clone)]
pub(super) struct SimConfig {
    /// The interpreter and its leading arguments (`["python3"]`); the script follows.
    pub interpreter: Vec<String>,
    /// The program under test, as the script is to start it; each word is a [`Template`], so `{tree}` is the root
    /// of the copy.
    pub program: Vec<Template>,
    /// The seeds run: `seed-0` to `seed-<seeds - 1>`.
    pub seeds: u32,
    /// The most cases with findings that are run a second time (besides `seed-0`).
    pub sample: usize,
    /// For one case.
    pub timeout: Duration,
    /// For the whole run, the second runs included.
    pub budget: Duration,
}

/// The measure configuration of a project.
#[derive(Debug, Clone)]
pub(super) struct Config {
    pub copy_exclude: Vec<PathBuf>,
    /// Variable and value; the value is a [`Template`], so it may hold `{run}` (the run's directory) and `{tree}`.
    pub env: Vec<(String, Template)>,
    pub build: Vec<String>,
    /// Without it nothing is mutated (a project that is only simulated).
    pub test: Option<Vec<String>>,
    /// Language to the command that makes its mutants.
    pub mutants: BTreeMap<String, Vec<String>>,
    /// Language to the command that finds the Items of an Interface from its code.
    pub items: BTreeMap<String, Vec<String>>,
    /// Context name to how its sim script is run.
    pub sim: BTreeMap<String, SimConfig>,
    pub layout: Vec<Layout>,
    /// For one build or one test of a mutant.
    pub timeout: Duration,
    /// For the build and the test before any change.
    pub baseline_timeout: Duration,
    /// For the whole run.
    pub budget: Duration,
    pub max_mutants: usize,
    pub seed: u64,
    /// The operator classes the provider is asked for; `None` leaves the choice to it (all it makes). Part of
    /// the digest, as the whole table is.
    pub operators: Option<Vec<String>>,
    /// Digest of the table as written: any change of it is a change of the settings of a run.
    pub digest: String,
}

impl Config {
    /// The place and language (if any) of the Interface `name` of `context`.
    pub(super) fn interface(&self, context: &str, name: &str) -> Option<(&Layout, Option<&str>)> {
        self.layout.iter().find_map(|l| match &l.place {
            Place::Interface { lang } if l.context == context && l.name == name => {
                Some((l, lang.as_deref()))
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

/// A step of the path to a value of the table.
enum Step {
    Key(String),
    Index(usize),
}

/// The path as the file's reader would name the field: `layout[0].path`, `sim.ledger.program[1]`.
fn named(trail: &[Step]) -> String {
    let mut path = String::new();
    for step in trail {
        match step {
            Step::Key(k) if path.is_empty() => path.push_str(k),
            Step::Key(k) => {
                path.push('.');
                path.push_str(k);
            }
            Step::Index(i) => path.push_str(&format!("[{i}]")),
        }
    }
    path
}

/// Whether the value at `trail` is a [`Template`]: the value of an `env` variable, or a word of the `program` of a
/// sim. The one list of the places that take a placeholder; the types in [`Config`] are the other, and a test keeps
/// them together.
fn takes_placeholders(trail: &[Step]) -> bool {
    match trail {
        [Step::Key(env), Step::Key(_)] => env == "env",
        [
            Step::Key(sim),
            Step::Key(_),
            Step::Key(program),
            Step::Index(_),
        ] => sim == "sim" && program == "program",
        _ => false,
    }
}

/// The error for `text` at `trail`: `{work}` anywhere, or `{run}` or `{tree}` where no placeholder is taken.
fn refuse_placeholder(text: &str, trail: &[Step], takes: bool) -> Result<(), String> {
    if text.contains(RETIRED) {
        return Err(format!(
            "`{}` has `{RETIRED}`, which is not a placeholder any more: use `{RUN}` (the run's directory) or `{TREE}` (the root of the copy)",
            named(trail)
        ));
    }
    if !takes && (text.contains(RUN) || text.contains(TREE)) {
        return Err(format!(
            "`{}` takes no placeholder (`{RUN}`, `{TREE}`): only the value of an `env` variable and a word of `sim.<context>.program` do",
            named(trail)
        ));
    }
    Ok(())
}

/// Walks the table before it is read into its types, and refuses `{work}` anywhere and `{run}` or `{tree}` in any
/// string or key but the values of [`takes_placeholders`]. A field added to the table is therefore refused a
/// placeholder until it is listed there.
fn check_placeholders(value: &toml::Value, trail: &mut Vec<Step>) -> Result<(), String> {
    match value {
        toml::Value::String(text) => refuse_placeholder(text, trail, takes_placeholders(trail)),
        toml::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                trail.push(Step::Index(i));
                check_placeholders(item, trail)?;
                trail.pop();
            }
            Ok(())
        }
        toml::Value::Table(table) => {
            for (key, item) in table {
                trail.push(Step::Key(key.clone()));
                refuse_placeholder(key, trail, false)?;
                check_placeholders(item, trail)?;
                trail.pop();
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The configuration the table `value` (the `[measure]` of the file) states.
pub(super) fn parse(value: &toml::Value) -> Result<Config, String> {
    check_placeholders(value, &mut Vec::new())?;
    let raw: Raw = value
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())?;
    command("build", &raw.build)?;
    if let Some(test) = &raw.test {
        command("test", test)?;
    }
    for (lang, words) in &raw.mutants {
        if lang.trim().is_empty() {
            return Err("a language in `mutants` has a name".to_string());
        }
        command(&format!("mutants.{lang}"), words)?;
    }
    for (lang, words) in &raw.items {
        if lang.trim().is_empty() {
            return Err("a language in `items` has a name".to_string());
        }
        command(&format!("items.{lang}"), words)?;
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
    if let Some(operators) = &raw.operators {
        if operators.is_empty() {
            return Err("`operators` names at least one class (leave it out for all)".to_string());
        }
        if let Some(bad) = operators
            .iter()
            .find(|o| Operator::try_from(o.as_str()).is_err())
        {
            return Err(format!(
                "`operators` has `{bad}`, which is not a class name (lowercase letters, digits, `-`)"
            ));
        }
    }
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
            env.into_iter().map(|(k, v)| (k, Template(v))).collect()
        }
        None => DEFAULT_ENV
            .iter()
            .map(|(k, v)| (k.to_string(), Template(v.to_string())))
            .collect(),
    };
    let mut sim = BTreeMap::new();
    for (context, s) in raw.sim {
        if context.trim().is_empty() {
            return Err("a context in `sim` has a name".to_string());
        }
        command(&format!("sim.{context}.interpreter"), &s.interpreter)?;
        command(&format!("sim.{context}.program"), &s.program)?;
        let seeds = match s.seeds {
            Some(0) => return Err(format!("`sim.{context}.seeds` must be at least 1")),
            Some(n) if n > MAX_SEEDS => {
                return Err(format!("`sim.{context}.seeds` is at most {MAX_SEEDS}"));
            }
            Some(n) => n,
            None => 50,
        };
        let sample = match s.sample {
            Some(n) if n > MAX_SAMPLE => {
                return Err(format!("`sim.{context}.sample` is at most {MAX_SAMPLE}"));
            }
            Some(n) => n,
            None => 6,
        };
        let sim_timeout = seconds(&format!("sim.{context}.timeout"), s.timeout, 60)?;
        let sim_budget = seconds(&format!("sim.{context}.budget"), s.budget, 600)?;
        if sim_budget.as_secs() > 3600 {
            return Err(format!("`sim.{context}.budget` is at most 3600 seconds"));
        }
        sim.insert(
            context,
            SimConfig {
                interpreter: s.interpreter,
                program: s.program.into_iter().map(Template).collect(),
                seeds,
                sample,
                timeout: sim_timeout,
                budget: sim_budget,
            },
        );
    }
    let mut layout = Vec::with_capacity(raw.layout.len());
    let mut seen = BTreeSet::new();
    for l in raw.layout {
        let path = inside("`layout` path", &l.path)?;
        let (name, place) = match (l.interface, l.harness, l.lang) {
            (Some(name), None, Some(lang)) => {
                if !raw.mutants.contains_key(&lang) && !raw.items.contains_key(&lang) {
                    return Err(format!(
                        "the layout of interface `{name}` has lang `{lang}`, which neither `mutants` nor `items` has a command for"
                    ));
                }
                (name, Place::Interface { lang: Some(lang) })
            }
            (Some(name), None, None) => (name, Place::Interface { lang: None }),
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
        items: raw.items,
        sim,
        layout,
        timeout,
        baseline_timeout,
        budget,
        max_mutants,
        seed: raw.seed.unwrap_or(0),
        operators: raw.operators,
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
            [(
                "CARGO_TARGET_DIR".to_string(),
                Template("{run}/target".to_string())
            )]
        );
        let (place, lang) = config.interface("inventory", "stock").unwrap();
        assert_eq!(
            (place.path.as_path(), lang),
            (Path::new("src/stock.rs"), Some("rust"))
        );
        assert_eq!(
            config.harness("inventory", "stock-tests").unwrap().path,
            Path::new("tests/stock.rs")
        );
        assert!(config.interface("inventory", "stock-tests").is_none());
        assert!(config.harness("other", "stock-tests").is_none());
    }

    #[test]
    fn a_language_with_a_command_in_items_alone_is_a_language_and_one_in_neither_is_not() {
        // the provider of items is read by the same loader, and is part of the digest
        let both = MINIMAL.replacen(
            "[mutants]",
            "[items]\nrust = [\"chidev\", \"provider\", \"items-rust\"]\n[mutants]",
            1,
        );
        let config = parse(&table(&both)).unwrap();
        assert_eq!(config.items["rust"], ["chidev", "provider", "items-rust"]);
        assert_ne!(config.digest, parse(&table(MINIMAL)).unwrap().digest);
        assert!(parse(&table(MINIMAL)).unwrap().items.is_empty());
        // a language that only `items` has a command for is a language of a layout
        let only_items = both.replacen(
            "[mutants]\nrust = [\"chidev\", \"provider\", \"mutants-rust\"]\n",
            "",
            1,
        );
        let config = parse(&table(&only_items)).unwrap();
        assert!(config.mutants.is_empty());
        let (_, lang) = config.interface("inventory", "stock").unwrap();
        assert_eq!(lang, Some("rust"));
        // and one that neither has a command for is refused
        let neither = only_items.replace("lang = \"rust\"", "lang = \"go\"");
        let error = parse(&table(&neither)).unwrap_err();
        assert!(error.contains("neither `mutants` nor `items`"), "{error}");
    }

    #[test]
    fn the_digest_follows_the_table_as_written() {
        let one = parse(&table(MINIMAL)).unwrap().digest;
        assert_eq!(one, parse(&table(MINIMAL)).unwrap().digest);
        let other = MINIMAL.replace("\"cargo\", \"test\"", "\"cargo\", \"test\", \"--quiet\"");
        assert_ne!(one, parse(&table(&other)).unwrap().digest);
    }

    #[test]
    fn operators_are_optional_and_part_of_the_digest() {
        let one = parse(&table(MINIMAL)).unwrap();
        assert_eq!(one.operators, None);
        let chosen = MINIMAL.replacen("[mutants]", "operators = [\"rel\", \"int\"]\n[mutants]", 1);
        let two = parse(&table(&chosen)).unwrap();
        assert_eq!(
            two.operators,
            Some(vec!["rel".to_string(), "int".to_string()])
        );
        assert_ne!(one.digest, two.digest);
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
            ("lang = \"rust\"", "lang = \"go\"", "neither `mutants` nor"),
            ("[mutants]", "[items]\nrust = []\n[mutants]", "items.rust"),
            (
                "[mutants]",
                "[items]\n\" \" = [\"x\"]\n[mutants]",
                "`items`",
            ),
            ("[mutants]", "timeout = 0\n[mutants]", "positive"),
            ("[mutants]", "budget = 7200\n[mutants]", "3600"),
            ("[mutants]", "max_mutants = 0\n[mutants]", "at least 1"),
            (
                "[mutants]",
                "operators = []\n[mutants]",
                "at least one class",
            ),
            (
                "[mutants]",
                "operators = [\"Str\"]\n[mutants]",
                "not a class name",
            ),
        ] {
            let error = parse(&table(&MINIMAL.replacen(from, to, 1))).unwrap_err();
            assert!(error.contains(expected), "{error} vs {expected}");
        }
    }

    const SIM: &str = r#"
build = ["python3", "-m", "py_compile", "prog.py"]
[sim.inventory]
interpreter = ["python3"]
program = ["python3", "{tree}/prog.py"]
[[layout]]
context = "inventory"
interface = "prog"
path = "prog.py"
[[layout]]
context = "inventory"
harness = "sweep-script"
path = "sim.py"
"#;

    #[test]
    fn a_sim_context_gets_its_defaults_and_a_project_that_is_only_simulated_needs_no_test_or_language()
     {
        let config = parse(&table(SIM)).unwrap();
        assert!(config.test.is_none());
        let sim = &config.sim["inventory"];
        assert_eq!(sim.interpreter, ["python3"]);
        assert_eq!(
            sim.program,
            ["python3", "{tree}/prog.py"].map(|w| Template(w.to_string()))
        );
        assert_eq!((sim.seeds, sim.sample), (50, 6));
        assert_eq!(sim.timeout, Duration::from_secs(60));
        assert_eq!(sim.budget, Duration::from_secs(600));
        let (place, lang) = config.interface("inventory", "prog").unwrap();
        assert_eq!((place.path.as_path(), lang), (Path::new("prog.py"), None));
        assert_eq!(
            config.harness("inventory", "sweep-script").unwrap().path,
            Path::new("sim.py")
        );
    }

    #[test]
    fn what_a_sim_cannot_run_with_is_refused_by_name() {
        for (from, to, expected) in [
            (
                "interpreter = [\"python3\"]",
                "interpreter = []",
                "sim.inventory.interpreter",
            ),
            (
                "program = [\"python3\", \"{tree}/prog.py\"]",
                "program = []",
                "sim.inventory.program",
            ),
            (
                "[sim.inventory]",
                "[sim.inventory]\nseeds = 0",
                "at least 1",
            ),
            (
                "[sim.inventory]",
                "[sim.inventory]\nseeds = 10001",
                "at most 10000",
            ),
            (
                "[sim.inventory]",
                "[sim.inventory]\nsample = 1001",
                "at most 1000",
            ),
            (
                "[sim.inventory]",
                "[sim.inventory]\ntimeout = 0",
                "positive",
            ),
            ("[sim.inventory]", "[sim.inventory]\nbudget = 3601", "3600"),
            ("[sim.inventory]", "[sim.inventory]\nseedz = 3", "seedz"),
        ] {
            let error = parse(&table(&SIM.replacen(from, to, 1))).unwrap_err();
            assert!(error.contains(expected), "{error} vs {expected}");
        }
        let tuned = SIM.replacen(
            "[sim.inventory]",
            "[sim.inventory]\nseeds = 8\nsample = 2\ntimeout = 5\nbudget = 90",
            1,
        );
        let sim = &parse(&table(&tuned)).unwrap().sim["inventory"];
        assert_eq!((sim.seeds, sim.sample), (8, 2));
        assert_eq!(
            (sim.timeout, sim.budget),
            (Duration::from_secs(5), Duration::from_secs(90))
        );
    }

    #[test]
    fn the_sim_table_is_part_of_the_digest() {
        let one = parse(&table(SIM)).unwrap().digest;
        let other = SIM.replacen("[sim.inventory]", "[sim.inventory]\nseeds = 9", 1);
        assert_ne!(one, parse(&table(&other)).unwrap().digest);
    }

    fn dirs() -> RunDirs {
        RunDirs::of(PathBuf::from("/p/.chidev/measure/r1"))
    }

    fn expanded(text: &str) -> String {
        dirs().expand(&Template(text.to_string()))
    }

    #[test]
    fn expand_makes_run_the_directory_and_tree_the_copy_and_leaves_other_braces() {
        assert_eq!(expanded("{run}/target"), "/p/.chidev/measure/r1/target");
        assert_eq!(
            expanded("{tree}/bank.py"),
            "/p/.chidev/measure/r1/tree/bank.py"
        );
        assert_eq!(
            expanded("{tree}:{run}"),
            "/p/.chidev/measure/r1/tree:/p/.chidev/measure/r1"
        );
        // any other brace stays as it is, and what was put in is not expanded again
        assert_eq!(expanded("{x} {{ { {tree"), "{x} {{ { {tree");
        let odd = RunDirs::of(PathBuf::from("/a{tree}b"));
        assert_eq!(odd.expand(&Template("{run}".to_string())), "/a{tree}b");
    }

    #[test]
    fn in_the_second_copy_of_a_sim_tree_is_rerun_tree_and_run_is_the_same_directory() {
        let first = dirs();
        let second = first.rerun();
        assert_eq!(second.dir, first.dir);
        let tree = |d: &RunDirs| d.expand(&Template("{tree}".to_string()));
        assert_eq!(tree(&first), "/p/.chidev/measure/r1/tree");
        assert_eq!(tree(&second), "/p/.chidev/measure/r1/rerun-tree");
        assert_eq!(
            second.expand(&Template("{run}/target".to_string())),
            "/p/.chidev/measure/r1/target"
        );
    }

    #[test]
    fn work_is_refused_everywhere_with_the_field_and_the_two_names_that_replace_it() {
        for (from, to, field) in [
            (
                "build = [",
                "env = { CARGO_TARGET_DIR = \"{work}/target\" }\nbuild = [",
                "env.CARGO_TARGET_DIR",
            ),
            (
                "build = [\"cargo\", \"build\", \"--tests\"]",
                "build = [\"cargo\", \"{work}\"]",
                "build[1]",
            ),
            (
                "path = \"src/stock.rs\"",
                "path = \"{work}/s.rs\"",
                "layout[0].path",
            ),
            (
                "[mutants]",
                "[mutants]\n\"{work}\" = [\"x\"]\n",
                "mutants.{work}",
            ),
        ] {
            let error = parse(&table(&MINIMAL.replacen(from, to, 1))).unwrap_err();
            assert!(error.contains(&format!("`{field}`")), "{error} vs {field}");
            assert!(
                error.contains("`{run}`") && error.contains("`{tree}`"),
                "{error}"
            );
            assert!(
                error.contains("run's directory") && error.contains("root of the copy"),
                "{error}"
            );
        }
        let error = parse(&table(&SIM.replacen("{tree}", "{work}", 1))).unwrap_err();
        assert!(error.contains("`sim.inventory.program[1]`"), "{error}");
        assert!(
            error.contains("`{run}`") && error.contains("`{tree}`"),
            "{error}"
        );
    }

    #[test]
    fn a_placeholder_is_refused_where_none_is_taken_and_the_field_is_named() {
        for (from, to, field) in [
            (
                "\"build\", \"--tests\"]",
                "\"build\", \"--tests\", \"{run}/x\"]",
                "build[3]",
            ),
            (
                "\"cargo\", \"test\"]",
                "\"cargo\", \"test\", \"{tree}\"]",
                "test[2]",
            ),
            (
                "\"mutants-rust\"]",
                "\"mutants-rust\", \"{run}\"]",
                "mutants.rust[3]",
            ),
            (
                "[mutants]",
                "[items]\nrust = [\"chidev\", \"{tree}/p\"]\n[mutants]",
                "items.rust[1]",
            ),
            (
                "path = \"src/stock.rs\"",
                "path = \"{run}/stock.rs\"",
                "layout[0].path",
            ),
            (
                "build = [",
                "copy_exclude = [\".git\", \"{tree}\"]\nbuild = [",
                "copy_exclude[1]",
            ),
            (
                "[mutants]",
                "[items]\n\"{run}\" = [\"x\"]\n[mutants]",
                "items.{run}",
            ),
            (
                "build = [",
                "env = { \"{tree}\" = \"x\" }\nbuild = [",
                "env.{tree}",
            ),
            (
                "build = [",
                "env = { K = [\"{run}\"] }\nbuild = [",
                "env.K[0]",
            ),
        ] {
            let error = parse(&table(&MINIMAL.replacen(from, to, 1))).unwrap_err();
            assert!(error.contains(&format!("`{field}`")), "{error} vs {field}");
            assert!(error.contains("takes no placeholder"), "{error}");
            assert!(
                error.contains("`env` variable") && error.contains("sim.<context>.program"),
                "{error}"
            );
        }
        let error = parse(&table(&SIM.replacen(
            "interpreter = [\"python3\"]",
            "interpreter = [\"python3\", \"{tree}\"]",
            1,
        )))
        .unwrap_err();
        assert!(error.contains("`sim.inventory.interpreter[1]`"), "{error}");
        assert!(error.contains("takes no placeholder"), "{error}");
        let error = parse(&table(&SIM.replacen(
            "[sim.inventory]",
            "[sim.\"{run}\"]\ninterpreter = [\"python3\"]\nprogram = [\"x\"]\n[sim.inventory]",
            1,
        )))
        .unwrap_err();
        assert!(error.contains("`sim.{run}`"), "{error}");
    }

    #[test]
    fn a_table_without_env_has_the_digest_of_the_table_alone() {
        // the default env is not in the table, so it is not in the digest either
        let value = table(MINIMAL);
        assert!(value.get("env").is_none());
        assert_eq!(
            parse(&value).unwrap().digest,
            Digest::of(&serde_json::to_vec(&value).unwrap()).to_string()
        );
    }

    #[test]
    fn the_places_that_take_a_placeholder_are_the_ones_that_are_expanded() {
        // the walk that allows a placeholder and the types that are expanded must name the same two places
        let text = SIM
            .replacen(
                "build = ",
                "env = { TARGET = \"{run}/t\", LINK = \"{tree}/l/{x}\" }\nbuild = ",
                1,
            )
            .replace(
                "\"{tree}/prog.py\"",
                "\"{tree}/prog.py\", \"{run}/out\", \"{y}\"",
            );
        let config = parse(&table(&text)).unwrap();
        let env: Vec<(&str, String)> = config
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), dirs().expand(v)))
            .collect();
        assert_eq!(
            env,
            [
                ("LINK", "/p/.chidev/measure/r1/tree/l/{x}".to_string()),
                ("TARGET", "/p/.chidev/measure/r1/t".to_string()),
            ]
        );
        let program: Vec<String> = config.sim["inventory"]
            .program
            .iter()
            .map(|w| dirs().expand(w))
            .collect();
        assert_eq!(
            program,
            [
                "python3",
                "/p/.chidev/measure/r1/tree/prog.py",
                "/p/.chidev/measure/r1/out",
                "{y}"
            ]
        );
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
