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
//! The first word of a program under test (a simulation's, or the examples') is digested by its file only when it is
//! a file of the host ([`ProgramStart`]); one that is in the copy is the build's and the pinned code's to account for.
//! These are not in the digest, for they are not settings: the environment the runner inherits from its host, and
//! `PATH`; the files of the copied tree other than the code pinned and the Harnesses written over it; and any
//! program started by a word after the first of a command (`sh -c "cargo test"` digests `sh`, and a rustup proxy
//! such as `cargo` is the same file whichever toolchain it starts). A change of a toolchain is therefore in general
//! not seen by the settings.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use super::config::{Config, Layout, Template, has_place};
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

/// What is said of a program of the role `role` that could not be found, in the words of the run's error.
fn refused(role: CommandRole, why: NotFound) -> RunnerError {
    match why {
        NotFound::Locate(f) => RunnerError::Locate {
            what: role,
            reason: f.message,
        },
        NotFound::Fingerprint(f) => RunnerError::Fingerprint {
            what: role,
            reason: f.message,
        },
    }
}

/// The error of a command with no program. Config refuses one, so there is always a first word; a command without
/// one names no file, which is what is said of it.
fn no_program(role: CommandRole) -> RunnerError {
    RunnerError::Locate {
        what: role,
        reason: "the command names no program".to_string(),
    }
}

/// The program `found`, as the project's root `root` has it.
fn executable_at(root: &Path, found: &Found) -> Executable {
    let at = found
        .executable
        .strip_prefix(root)
        .unwrap_or(&found.executable);
    Executable {
        at: at.to_string_lossy().into_owned(),
        digest: found.digest.clone(),
    }
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
        let Some((program, args)) = words.split_first() else {
            return Err(no_program(role));
        };
        let found = find(root, program).await.map_err(|e| refused(role, e))?;
        Ok(Self {
            program: program.clone(),
            args: args.to_vec(),
            executable: executable_at(root, &found),
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

/// Where the first word of the program under test (of a simulation or of the examples) starts it from, which says
/// whether the settings can hold its file.
///
/// The build, the test command, the provider and the interpreter are digested by their files even when the file is
/// inside the project's root, for they exist before the build runs. The program under test is not like them: it may
/// be what the build writes (`./target/debug/app`, `{tree}/bin/app`), and `target` is not even copied. A file at the
/// root under that path is then a local build, not the program the run starts, and digesting it would stale a run
/// for a rebuild on the developer's machine, or leave the settings unreadable for good where nothing is built.
/// So the word is sorted by where it resolves when the run starts:
///
/// - a word with `{tree}` or `{run}`, a relative path (it has a `/`), an absolute path inside the project's root,
///   and a bare name that `PATH` resolves to a file inside the root (a relative entry of it) are [`InCopy`]: the
///   file is in the copy the run makes, and the pinned code and the build account for it;
/// - any other is [`Host`]: a file that is outside the project (`python3`, `/usr/bin/env`), which the copy resolves
///   to as the root does. Its file is digested.
///
/// What a run of the kind has to say of the words as written is in the settings either way.
///
/// [`InCopy`]: ProgramStart::InCopy
/// [`Host`]: ProgramStart::Host
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) enum ProgramStart {
    /// A file of the host, outside the project, with its executable as found now. The same file is found from the
    /// copy, so nothing is checked against it when the copy is made.
    Host(Executable),
    /// A file of the copy the run makes (it may be written by the build): not digested.
    InCopy,
}

impl ProgramStart {
    /// Where the `word` of the role `role` starts from, for the project rooted at `root`; the executable of a
    /// [`ProgramStart::Host`] is found as [`find`] finds a program, and digested. An error when it cannot be found or
    /// read; one that is in the copy is neither looked for nor read.
    pub(super) async fn of(
        root: &Path,
        role: CommandRole,
        word: &str,
    ) -> Result<Self, RunnerError> {
        let search = std::env::var_os("PATH").unwrap_or_default();
        Self::searching(root, role, word, &search).await
    }

    /// As [`ProgramStart::of`], with `search` in the place of `PATH`.
    async fn searching(
        root: &Path,
        role: CommandRole,
        word: &str,
        search: &OsStr,
    ) -> Result<Self, RunnerError> {
        let path = Path::new(word);
        let in_copy = has_place(word)
            || (path.is_absolute() && path.starts_with(root))
            || (!path.is_absolute() && word.contains('/'));
        if in_copy {
            return Ok(Self::InCopy);
        }
        let located = process::locate_in(root, word, search)
            .await
            .map_err(|f| refused(role.clone(), NotFound::Locate(f)))?;
        // A bare name found through a relative entry of `PATH` is a file of the project, which the copy has.
        if located.starts_with(root) {
            return Ok(Self::InCopy);
        }
        let digest = process::file_digest(&located)
            .await
            .map_err(|f| refused(role, NotFound::Fingerprint(f)))?;
        Ok(Self::Host(executable_at(
            root,
            &Found {
                executable: located,
                digest,
            },
        )))
    }
}

/// The program under test of a simulation or of the examples of a Context: where its first word starts it from
/// ([`ProgramStart`]) and its words as written, each a [`Template`] that is made the places of a run when it is
/// started.
///
/// A [`ProgramStart::Host`] file is in the settings digest, and the copy resolves to the file the root does, so
/// there is nothing to check against it when the copy is made (`settings_moved` is not for it). An
/// [`ProgramStart::InCopy`] one has no digest to hold the copy to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ProgramUnderTest {
    start: ProgramStart,
    program: Template,
    args: Vec<Template>,
}

impl ProgramUnderTest {
    /// The program `words` of `role`, started from where its first word says, for the project rooted at `root`.
    async fn resolve(
        root: &Path,
        role: CommandRole,
        words: &[Template],
    ) -> Result<Self, RunnerError> {
        let Some((program, args)) = words.split_first() else {
            return Err(no_program(role));
        };
        Ok(Self {
            start: ProgramStart::of(root, role, program.as_str()).await?,
            program: program.clone(),
            args: args.to_vec(),
        })
    }

    /// The first word, as written: the program.
    pub(super) fn program(&self) -> &Template {
        &self.program
    }

    /// The words after the first, as written.
    pub(super) fn args(&self) -> &[Template] {
        &self.args
    }

    /// The words, as written.
    pub(super) fn words(&self) -> impl Iterator<Item = &Template> {
        std::iter::once(&self.program).chain(&self.args)
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
/// program the script drives (the words as written, and where the first starts from, [`ProgramStart`]), the cases
/// and the limits, and where the script and the code of the Interfaces it builds are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SimSettings {
    pub copy: CopySettings,
    /// Runs the script.
    pub interpreter: Program,
    /// The program under test, as the script is to start it, each word as written, and where it starts from.
    pub program: ProgramUnderTest,
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
            program: ProgramUnderTest::resolve(root, CommandRole::SimProgram, &sim.program).await?,
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
/// run against (the words as written, and where the first starts from, [`ProgramStart`]), the limits, and where
/// the code of the Interfaces it builds is written. The examples are not here: they are the model's, and a change of
/// one moves the Harness's revision and not the settings. Nor is the Harness: a Harness of examples has no file, the
/// run reads and writes nothing at a place of it, and so it needs no entry of the layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ExamplesSettings {
    pub copy: CopySettings,
    /// Starts the program under test; the arguments of an example follow its own, each word as written.
    pub program: ProgramUnderTest,
    /// For one example.
    pub timeout: Duration,
    /// For the whole run.
    pub budget: Duration,
    /// Where the code of each Interface the Harness builds is written, sorted by name.
    pub builds: Vec<Placed>,
}

impl ExamplesSettings {
    /// The settings of a run of the examples of a Harness of `context` that builds the Interfaces `builds`, from
    /// `config`, its programs found at the project's root `root`. An error when the table cannot run it.
    pub(super) async fn resolve(
        config: &Config,
        root: &Path,
        context: &ContextName,
        builds: &[AssetName],
    ) -> Result<Self, RunnerError> {
        let examples =
            config
                .examples
                .get(context)
                .ok_or_else(|| RunnerError::NoExamplesConfig {
                    context: context.clone(),
                })?;
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
            program: ProgramUnderTest::resolve(
                root,
                CommandRole::ExamplesProgram,
                &examples.program,
            )
            .await?,
            timeout: examples.timeout,
            budget: examples.budget,
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

    /// Writes the executable `at` (a path under `root`, whose directory is made) with `text` as its script.
    fn executable(root: &Path, at: &str, text: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let file = root.join(at);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    /// Where the `word` of a program under test starts from in the project rooted at `root`, when `PATH` is `search`.
    async fn start_of(root: &Path, word: &str, search: &str) -> Result<ProgramStart, RunnerError> {
        ProgramStart::searching(root, CommandRole::SimProgram, word, OsStr::new(search)).await
    }

    #[tokio::test]
    async fn a_word_starts_from_the_host_only_when_it_resolves_to_a_file_outside_the_project() {
        let root = dir("start-host");
        let outside = dir("start-host-outside");
        let tool = executable(&outside, "tool", "#!/bin/sh\n");
        let search = outside.to_string_lossy().into_owned();
        // a bare name that `PATH` resolves outside the root, and a path outside it: the host's
        let by_name = start_of(&root, "tool", &search).await.unwrap();
        let by_path = start_of(&root, &tool.to_string_lossy(), "").await.unwrap();
        for start in [&by_name, &by_path] {
            let ProgramStart::Host(found) = start else {
                panic!("{start:?} is not the host's");
            };
            // outside the root, so held as it is found; and digested as the file is
            assert_eq!(Path::new(&found.at), tool);
            assert_eq!(found.digest, process::file_digest(&tool).await.unwrap());
        }
        assert_eq!(by_name, by_path);
        // `sh` and `env` are found on any unix host, `python3` or not
        assert!(matches!(
            start_of(&root, "sh", &std::env::var("PATH").unwrap())
                .await
                .unwrap(),
            ProgramStart::Host(_)
        ));
        assert!(matches!(
            start_of(&root, "/usr/bin/env", "").await.unwrap(),
            ProgramStart::Host(_)
        ));
        // the file changed, the executable it is held as changes
        executable(&outside, "tool", "#!/bin/sh\n# changed\n");
        assert_ne!(start_of(&root, "tool", &search).await.unwrap(), by_name);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[tokio::test]
    async fn a_word_starts_in_the_copy_when_it_names_a_place_a_relative_path_or_a_file_inside_the_project()
     {
        let root = dir("start-copy");
        // none of these is looked for or read: the file is the copy's, and may be what the build writes later
        let inside = root.join("out/app").to_string_lossy().into_owned();
        for word in [
            "./target/debug/app",
            "bin/app",
            "{tree}/x",
            "{tree}",
            "{run}/target/debug/app",
            inside.as_str(),
        ] {
            assert_eq!(
                start_of(&root, word, "").await.unwrap(),
                ProgramStart::InCopy,
                "{word}"
            );
        }
        // nor is a file at that path read when there is one: a directory is no fingerprint failure
        std::fs::create_dir_all(root.join("target/debug/app")).unwrap();
        assert_eq!(
            start_of(&root, "./target/debug/app", "").await.unwrap(),
            ProgramStart::InCopy
        );
        // a bare name that `PATH` resolves through a relative entry is a file of the project
        executable(&root, "bin/prog", "#!/bin/sh\n");
        assert_eq!(
            start_of(&root, "prog", "bin").await.unwrap(),
            ProgramStart::InCopy
        );
        // an entry that is absolute and inside the root is the project's too
        assert_eq!(
            start_of(&root, "prog", &root.join("bin").to_string_lossy())
                .await
                .unwrap(),
            ProgramStart::InCopy
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_host_program_that_is_not_there_or_not_a_file_says_whose_it_is() {
        let root = dir("start-missing");
        let error = start_of(&root, "chidev-no-such-program", "")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "the program of the simulation: executable `chidev-no-such-program` not found in PATH"
        );
        assert!(matches!(error, RunnerError::Locate { .. }));
        let error = ProgramStart::searching(
            &root,
            CommandRole::ExamplesProgram,
            "/chidev-no-such-dir/app",
            OsStr::new(""),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                &error,
                RunnerError::Locate {
                    what: CommandRole::ExamplesProgram,
                    ..
                }
            ),
            "{error:?}"
        );
        // found, and it is a directory: its bytes cannot be read
        let outside = dir("start-missing-outside");
        let error = start_of(&root, &outside.to_string_lossy(), "")
            .await
            .unwrap_err();
        assert!(
            matches!(&error, RunnerError::Fingerprint { .. }),
            "{error:?}"
        );
        // a name that `PATH` finds nowhere, with a relative entry that has no such file
        let error = start_of(&root, "prog", "bin").await.unwrap_err();
        assert!(matches!(error, RunnerError::Locate { .. }), "{error:?}");
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[tokio::test]
    async fn a_program_under_test_holds_its_words_as_written_and_where_the_first_starts_from() {
        let root = dir("under-test");
        let templates =
            |list: &[&str]| -> Vec<Template> { serde_json::from_value(list.into()).unwrap() };
        let under = ProgramUnderTest::resolve(
            &root,
            CommandRole::ExamplesProgram,
            &templates(&["./target/debug/app", "{tree}/x", "-v"]),
        )
        .await
        .unwrap();
        assert_eq!(under.start, ProgramStart::InCopy);
        assert_eq!(under.program().as_str(), "./target/debug/app");
        assert_eq!(
            under
                .args()
                .iter()
                .map(Template::as_str)
                .collect::<Vec<_>>(),
            ["{tree}/x", "-v"]
        );
        assert_eq!(
            under.words().map(Template::as_str).collect::<Vec<_>>(),
            ["./target/debug/app", "{tree}/x", "-v"]
        );
        // where it starts from is in what is digested, with the words as written
        let json = serde_json::to_string(&under).unwrap();
        assert!(
            json.contains("\"InCopy\"") && json.contains("{tree}/x"),
            "{json}"
        );
        let error = ProgramUnderTest::resolve(&root, CommandRole::SimProgram, &[])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::Locate {
                what: CommandRole::SimProgram,
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
