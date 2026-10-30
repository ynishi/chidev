//! What a run reads of the `[measure]` table, resolved: the values with their defaults applied, the places the
//! run writes code to, and the programs it starts with the digests of their executables. There is one struct per
//! kind of run ([`MutationSettings`], [`SimSettings`], [`ExamplesSettings`]), and the run is made from it and from nothing else of the
//! table, so a setting the run does not read is not in the struct, and the digest of the struct, which is the
//! settings digest of the measurement, cannot move for it. A setting the run reads is a field of the struct, and
//! the digest moves with it.
//!
//! The digest is that of the struct's value written as JSON. Lists whose order means nothing (the layout, the
//! entries to leave out of a copy) are sorted and maps are `BTreeMap`s, so the same settings give the same digest
//! however the table orders them, and a table with a default written out gives the digest of one without it.
//!
//! These are not in the digest, for they are not settings: the environment the runner inherits from its host, and
//! `PATH`; the files of the copied tree other than the code pinned and the Harnesses written over it; and any
//! program started by a word after the first of a command (`sh -c "cargo test"` digests `sh`, and a rustup proxy
//! such as `cargo` is the same file whichever toolchain it starts). A change of a toolchain is therefore in general
//! not seen by the settings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use super::config::{Config, Layout, Template};
use crate::app::ports::measure::{CommandRole, LayoutPlace, RunnerError};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::measure::SettingsDigest;
use crate::domain::vo::name::{AssetName, ContextName};
use crate::infra::process;

/// A program found: where, as the project's root has it, and the digest of its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Executable {
    /// Relative to the project root when it is inside it, so that moving the project does not move the settings;
    /// otherwise as it is, absolute.
    at: String,
    /// The SHA-256 of the file, in lower-case hex.
    digest: String,
}

impl Executable {
    /// The digest of the file.
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }
}

/// A program as a tree has it: the file the command names, and the digest of that file. The file is not followed
/// through its links, for a program that is a link (a rustup proxy) has to be started by the link's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Found {
    /// The file, as found.
    pub executable: PathBuf,
    /// The SHA-256 of the file, in lower-case hex.
    pub digest: String,
}

/// Why a program could not be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NotFound {
    /// No file is named by it.
    Locate(process::Failure),
    /// The file is there, and its bytes cannot be read.
    Fingerprint(process::Failure),
}

impl NotFound {
    /// The failure of the process module that it is.
    pub(super) fn failure(self) -> process::Failure {
        match self {
            NotFound::Locate(f) | NotFound::Fingerprint(f) => f,
        }
    }
}

/// Finds `program` in the tree rooted at `base`, by the one rule: the project's root for the settings, and the
/// copy of the project for the run. A path is taken from `base`, an absolute one stays, and a bare name is looked
/// for in `PATH`, an entry of which that is relative being taken from `base` too.
pub(super) async fn find(base: &Path, program: &str) -> Result<Found, NotFound> {
    let executable = process::locate(base, program)
        .await
        .map_err(NotFound::Locate)?;
    let digest = process::file_digest(&executable)
        .await
        .map_err(NotFound::Fingerprint)?;
    Ok(Found { executable, digest })
}

/// A command of the table: the program its first word names, its arguments, and the executable of the program as
/// the project's root has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Program {
    program: String,
    args: Vec<String>,
    executable: Executable,
}

impl Program {
    /// The command `words` of `role`, its program found at the project's root `root`.
    async fn resolve(
        root: &Path,
        role: CommandRole,
        words: &[String],
    ) -> Result<Self, RunnerError> {
        // Config refuses a command with no program, so there is always a first word; a command without one names
        // no file, which is what is said of it.
        let Some((program, args)) = words.split_first() else {
            return Err(RunnerError::Locate {
                what: role,
                reason: "the command names no program".to_string(),
            });
        };
        let found = find(root, program).await.map_err(|e| match e {
            NotFound::Locate(f) => RunnerError::Locate {
                what: role,
                reason: f.message,
            },
            NotFound::Fingerprint(f) => RunnerError::Fingerprint {
                what: role,
                reason: f.message,
            },
        })?;
        let at = found
            .executable
            .strip_prefix(root)
            .unwrap_or(&found.executable);
        Ok(Self {
            program: program.clone(),
            args: args.to_vec(),
            executable: Executable {
                at: at.to_string_lossy().into_owned(),
                digest: found.digest,
            },
        })
    }

    /// The program, as the command names it.
    pub(super) fn program(&self) -> &str {
        &self.program
    }

    /// The arguments after the program.
    pub(super) fn args(&self) -> &[String] {
        &self.args
    }

    /// The program and its arguments.
    pub(super) fn words(&self) -> Vec<String> {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect()
    }

    /// The executable as the project's root has it.
    pub(super) fn executable(&self) -> &Executable {
        &self.executable
    }
}

/// Where the code of an Interface or the source of a Harness is written in the copy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(super) struct Placed {
    name: String,
    /// Relative to the project root, without `..`.
    path: String,
}

impl Placed {
    fn of(name: &AssetName, layout: &Layout) -> Self {
        Self {
            name: name.to_string(),
            path: layout.path.to_string_lossy().into_owned(),
        }
    }

    /// Where it is written, relative to the root of the copy.
    pub(super) fn path(&self) -> &Path {
        Path::new(&self.path)
    }
}

/// The place of the one called `name` among `placed`.
fn place_of<'a>(placed: &'a [Placed], name: &AssetName) -> Option<&'a Path> {
    placed
        .iter()
        .find(|p| p.name == name.as_str())
        .map(Placed::path)
}

fn no_layout(place: LayoutPlace, context: &ContextName, name: &AssetName) -> RunnerError {
    RunnerError::NoLayout {
        place,
        context: context.clone(),
        name: name.clone(),
    }
}

/// What every run reads: how the project is copied, in what environment the commands run, how the copy is built,
/// and how long the build before any change may take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct CopySettings {
    /// Paths relative to the project root that a copy leaves out. A set: the order they are written in means
    /// nothing.
    pub copy_exclude: BTreeSet<String>,
    /// Variable and value, the value as written: `{run}` and `{tree}` in it are made the places of a run when it
    /// is started, so the value is not the same text twice.
    pub env: BTreeMap<String, Template>,
    /// Builds the copy.
    pub build: Program,
    /// For the build and the test before any change.
    pub baseline_timeout: Duration,
}

impl CopySettings {
    async fn resolve(config: &Config, root: &Path) -> Result<Self, RunnerError> {
        Ok(Self {
            copy_exclude: config
                .copy_exclude
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            env: config.env.iter().cloned().collect(),
            build: Program::resolve(root, CommandRole::Build, &config.build).await?,
            baseline_timeout: config.baseline_timeout,
        })
    }

    /// The paths a copy leaves out.
    pub(super) fn excluded(&self) -> Vec<PathBuf> {
        self.copy_exclude.iter().map(PathBuf::from).collect()
    }
}

/// The digest of `settings` written as JSON. It errs when they cannot be written as JSON.
fn digest_of<T: Serialize>(settings: &T) -> Result<SettingsDigest, RunnerError> {
    let json = serde_json::to_vec(settings).map_err(|e| RunnerError::Digest(e.to_string()))?;
    Ok(SettingsDigest::new(Digest::of(&json)))
}

/// What a mutation run of an Interface reads: how it copies, builds and tests, the provider that makes the mutants
/// for the language of the Interface, the limits on the run and on one mutant, and where the code and the tests it
/// runs are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct MutationSettings {
    pub copy: CopySettings,
    /// Runs the tests.
    pub test: Program,
    /// The language of the Interface, which picks the provider.
    pub lang: String,
    /// Makes the mutants.
    pub provider: Program,
    /// The operator classes asked of the provider; none leaves the choice to it.
    pub operators: Option<Vec<String>>,
    /// For one build or one test of a mutant.
    pub timeout: Duration,
    /// For the whole run.
    pub budget: Duration,
    pub max_mutants: usize,
    pub seed: u64,
    /// Where the code of the Interface is written.
    pub interface: Placed,
    /// Where the source of each Harness the run runs is written, sorted by name.
    pub harnesses: Vec<Placed>,
}

impl MutationSettings {
    /// The settings of a mutation run of `interface` of `context` that runs the tests `harnesses`, from `config`,
    /// its programs found at the project's root `root`. An error when the table cannot mutate it.
    pub(super) async fn resolve(
        config: &Config,
        root: &Path,
        context: &ContextName,
        interface: &AssetName,
        harnesses: &[AssetName],
    ) -> Result<Self, RunnerError> {
        let (place, lang) = config
            .interface(context, interface)
            .ok_or_else(|| no_layout(LayoutPlace::Interface, context, interface))?;
        let lang = lang.ok_or_else(|| RunnerError::NoLang {
            interface: interface.clone(),
        })?;
        let provider = config
            .mutants
            .get(lang)
            .ok_or_else(|| RunnerError::NoProvider {
                lang: lang.to_string(),
            })?;
        let test = config.test.as_deref().ok_or(RunnerError::NoTestCommand)?;
        let mut placed = Vec::with_capacity(harnesses.len());
        for name in harnesses {
            let layout = config
                .harness(context, name)
                .ok_or_else(|| no_layout(LayoutPlace::Harness, context, name))?;
            placed.push(Placed::of(name, layout));
        }
        placed.sort();
        Ok(Self {
            copy: CopySettings::resolve(config, root).await?,
            test: Program::resolve(root, CommandRole::Test, test).await?,
            lang: lang.to_string(),
            provider: Program::resolve(
                root,
                CommandRole::Provider {
                    lang: lang.to_string(),
                },
                provider,
            )
            .await?,
            operators: config.operators.clone(),
            timeout: config.timeout,
            budget: config.budget,
            max_mutants: config.max_mutants,
            seed: config.seed,
            interface: Placed::of(interface, place),
            harnesses: placed,
        })
    }

    /// The digest of the settings; an error when they cannot be written down to be digested.
    pub(super) fn digest(&self) -> Result<SettingsDigest, RunnerError> {
        digest_of(self)
    }

    /// Where the source of the Harness `name` is written; none for one the settings were not made for.
    pub(super) fn harness_path(&self, name: &AssetName) -> Option<&Path> {
        place_of(&self.harnesses, name)
    }
}

/// What a simulation of a sim Harness reads: how it copies and builds, the interpreter the script runs under, the
/// program the script drives, the cases and the limits, and where the script and the code of the Interfaces it
/// builds are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SimSettings {
    pub copy: CopySettings,
    /// Runs the script.
    pub interpreter: Program,
    /// The program under test, as the script is to start it, each word as written.
    pub program: Vec<Template>,
    /// The seeds run: `seed-0` to `seed-<seeds - 1>`.
    pub seeds: u32,
    /// The most cases with findings that are run a second time (besides `seed-0`).
    pub sample: usize,
    /// For one case.
    pub timeout: Duration,
    /// For the whole run, the second runs included.
    pub budget: Duration,
    /// Where the script is written.
    pub harness: Placed,
    /// Where the code of each Interface the Harness builds is written, sorted by name.
    pub builds: Vec<Placed>,
}

impl SimSettings {
    /// The settings of a simulation of the sim Harness `harness` of `context` that builds the Interfaces
    /// `builds`, from `config`, its programs found at the project's root `root`. An error when the table cannot
    /// simulate it.
    pub(super) async fn resolve(
        config: &Config,
        root: &Path,
        context: &ContextName,
        harness: &AssetName,
        builds: &[AssetName],
    ) -> Result<Self, RunnerError> {
        let sim = config
            .sim
            .get(context)
            .ok_or_else(|| RunnerError::NoSimConfig {
                context: context.clone(),
            })?;
        let script = config
            .harness(context, harness)
            .ok_or_else(|| no_layout(LayoutPlace::Harness, context, harness))?;
        let mut placed = Vec::with_capacity(builds.len());
        for name in builds {
            let (layout, _) = config
                .interface(context, name)
                .ok_or_else(|| no_layout(LayoutPlace::Interface, context, name))?;
            placed.push(Placed::of(name, layout));
        }
        placed.sort();
        Ok(Self {
            copy: CopySettings::resolve(config, root).await?,
            interpreter: Program::resolve(root, CommandRole::Interpreter, &sim.interpreter).await?,
            program: sim.program.clone(),
            seeds: sim.seeds,
            sample: sim.sample,
            timeout: sim.timeout,
            budget: sim.budget,
            harness: Placed::of(harness, script),
            builds: placed,
        })
    }

    /// The digest of the settings; an error when they cannot be written down to be digested.
    pub(super) fn digest(&self) -> Result<SettingsDigest, RunnerError> {
        digest_of(self)
    }

    /// Where the code of the Interface `name` is written; none for one the settings were not made for.
    pub(super) fn build_path(&self, name: &AssetName) -> Option<&Path> {
        place_of(&self.builds, name)
    }
}

/// What a run of the examples of a Harness of examples reads: how it copies and builds, the program the examples are
/// run against (the program its first word names is the one digested, the words as written), the limits, and where
/// the Harness is laid out and where the code of the Interfaces it builds is written. The examples are not here: they
/// are the model's, and a change of one moves the Harness's revision and not the settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ExamplesSettings {
    pub copy: CopySettings,
    /// Starts the program under test; the arguments of an example follow its own, each word as written.
    pub program: Program,
    /// For one example.
    pub timeout: Duration,
    /// For the whole run.
    pub budget: Duration,
    /// The Harness, as the layout names it. Nothing is written there: a Harness of examples has no code.
    pub harness: Placed,
    /// Where the code of each Interface the Harness builds is written, sorted by name.
    pub builds: Vec<Placed>,
}

impl ExamplesSettings {
    /// The settings of a run of the examples of the Harness `harness` of `context` that builds the Interfaces
    /// `builds`, from `config`, its programs found at the project's root `root`. An error when the table cannot run
    /// it.
    pub(super) async fn resolve(
        config: &Config,
        root: &Path,
        context: &ContextName,
        harness: &AssetName,
        builds: &[AssetName],
    ) -> Result<Self, RunnerError> {
        let examples =
            config
                .examples
                .get(context)
                .ok_or_else(|| RunnerError::NoExamplesConfig {
                    context: context.clone(),
                })?;
        let laid_out = config
            .harness(context, harness)
            .ok_or_else(|| no_layout(LayoutPlace::Harness, context, harness))?;
        let mut placed = Vec::with_capacity(builds.len());
        for name in builds {
            let (layout, _) = config
                .interface(context, name)
                .ok_or_else(|| no_layout(LayoutPlace::Interface, context, name))?;
            placed.push(Placed::of(name, layout));
        }
        placed.sort();
        let words: Vec<String> = examples
            .program
            .iter()
            .map(|w| w.as_str().to_string())
            .collect();
        Ok(Self {
            copy: CopySettings::resolve(config, root).await?,
            program: Program::resolve(root, CommandRole::Program, &words).await?,
            timeout: examples.timeout,
            budget: examples.budget,
            harness: Placed::of(harness, laid_out),
            builds: placed,
        })
    }

    /// The digest of the settings; an error when they cannot be written down to be digested.
    pub(super) fn digest(&self) -> Result<SettingsDigest, RunnerError> {
        digest_of(self)
    }

    /// Where the code of the Interface `name` is written; none for one the settings were not made for.
    pub(super) fn build_path(&self, name: &AssetName) -> Option<&Path> {
        place_of(&self.builds, name)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A new, empty directory for a test, under the system's temporary directory.
    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chidev-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn settings_that_cannot_be_written_as_json_cannot_be_digested() {
        // a map whose key is not a string has no JSON form
        let unwritable = BTreeMap::from([((1u8, 2u8), 3u8)]);
        let error = digest_of(&unwritable).unwrap_err();
        assert!(matches!(&error, RunnerError::Digest(_)), "{error:?}");
        assert!(
            error
                .to_string()
                .starts_with("the settings cannot be digested: "),
            "{error}"
        );
        assert!(digest_of(&BTreeMap::from([("a", 1u8)])).is_ok());
    }

    #[tokio::test]
    async fn a_command_with_no_word_names_no_program() {
        let root = dir("empty");
        let error = Program::resolve(&root, CommandRole::Build, &[])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::Locate {
                what: CommandRole::Build,
                reason: "the command names no program".to_string()
            }
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_program_in_the_project_is_held_relative_to_its_root_and_one_outside_as_it_is() {
        let root = dir("relative");
        std::fs::write(root.join("tool.sh"), "#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.join("tool.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let inside = Program::resolve(&root, CommandRole::Test, &words(&["./tool.sh", "-x"]))
            .await
            .unwrap();
        assert_eq!(inside.executable().at, "tool.sh");
        assert_eq!(inside.words(), words(&["./tool.sh", "-x"]));
        assert_eq!(inside.args(), ["-x"]);
        // the same file found from the copy of the project is found by the same rule, at its place there
        let copy = dir("relative-copy");
        std::fs::copy(root.join("tool.sh"), copy.join("tool.sh")).unwrap();
        let found = find(&copy, "./tool.sh").await.unwrap();
        assert_eq!(found.executable, copy.join("./tool.sh"));
        assert_eq!(found.digest, inside.executable().digest());
        // a program that is not in the project keeps the path it was found at
        let outside = Program::resolve(&root, CommandRole::Test, &words(&["sh"]))
            .await
            .unwrap();
        assert!(Path::new(&outside.executable().at).is_absolute());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(copy);
    }
}
