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
//! Settings are resolved for many subjects at once when the runner is asked so ([`Files`]): the programs they
//! share are found and hashed once, by the path of the file, and each command still keeps its own words, so the
//! same binary started with other arguments is another [`Program`] with the one digest of its file. That memo is
//! local to one resolution and is gone with it.
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

/// The digests of the files read while the settings of one or many subjects are resolved, so that a file that
/// several of them use is read once. It is keyed by the file's path as resolved, and holds only what reading the
/// file gave (its digest, or why its bytes cannot be read) and nothing of the command that names it: two commands
/// that start the same file with other arguments are each their own [`Program`], with the one digest of the file.
///
/// It is a value local to one resolution: whoever resolves makes one and drops it, so a file that changes between
/// two resolutions is read again. It covers the programs found through [`find_in`] (build, test, provider,
/// interpreter) and the file of a [`ProgramStart::Host`]; a program that is [`ProgramStart::InCopy`] or
/// [`ProgramStart::Unfound`] has no digest and is not in it.
#[derive(Debug, Default)]
pub(super) struct Files {
    digests: BTreeMap<PathBuf, Result<String, process::Failure>>,
    /// How many files were read, which a test counts to see that a shared file is read once.
    #[cfg(test)]
    read: usize,
}

impl Files {
    /// The digest of `file`, read the first time it is asked.
    async fn digest(&mut self, file: &Path) -> Result<String, process::Failure> {
        // the same file spelled with and without a `.` component is the same key
        let key: PathBuf = file.components().collect();
        if let Some(said) = self.digests.get(&key) {
            return said.clone();
        }
        #[cfg(test)]
        {
            self.read += 1;
        }
        let said = process::file_digest(file).await;
        self.digests.insert(key, said.clone());
        said
    }

    /// How many files were read.
    #[cfg(test)]
    pub(super) fn read(&self) -> usize {
        self.read
    }
}

/// Finds `program` in the tree rooted at `base`, by the one rule: the project's root for the settings, and the
/// copy of the project for the run. A path is taken from `base`, an absolute one stays, and a bare name is looked
/// for in `PATH`, an entry of which that is relative being taken from `base` too.
pub(super) async fn find(base: &Path, program: &str) -> Result<Found, NotFound> {
    find_in(&mut Files::default(), base, program).await
}

/// As [`find`], reading the file through `files`: a file it has read is not read again.
pub(super) async fn find_in(
    files: &mut Files,
    base: &Path,
    program: &str,
) -> Result<Found, NotFound> {
    let executable = process::locate(base, program)
        .await
        .map_err(NotFound::Locate)?;
    let digest = files
        .digest(&executable)
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

/// The program at `executable` whose file has `digest`, as the project's root `root` has it.
fn executable_at(root: &Path, executable: &Path, digest: String) -> Executable {
    let at = executable.strip_prefix(root).unwrap_or(executable);
    Executable {
        at: at.to_string_lossy().into_owned(),
        digest,
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
    /// The command `words` of `role`, its program found at the project's root `root`, its file read through `files`.
    async fn resolve(
        files: &mut Files,
        root: &Path,
        role: CommandRole,
        words: &[String],
    ) -> Result<Self, RunnerError> {
        let Some((program, args)) = words.split_first() else {
            return Err(no_program(role));
        };
        let found = find_in(files, root, program)
            .await
            .map_err(|e| refused(role, e))?;
        Ok(Self {
            program: program.clone(),
            args: args.to_vec(),
            executable: executable_at(root, &found.executable, found.digest),
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
/// So the word is sorted by where it resolves when the run starts it, with the copy as the working directory:
///
/// - a word with `{tree}` or `{run}`, a relative path (more than one component: `./target/debug/app`, `bin/app`),
///   and a bare name that the walk of `PATH` first finds under a relative entry (which is relative to the copy) are
///   [`InCopy`]: the file is in the copy the run makes, and the pinned code and the build account for it;
/// - an absolute path, wherever it points (inside the project's root too: at run time it is the root's file and
///   not the copy's), and a bare name that the walk first finds under an absolute entry, are [`Host`]: a file
///   outside the copy (`python3`, `/usr/bin/env`). Its file is digested, and the copy is asked before the run
///   whether the word still resolves to it ([`ProgramUnderTest::moved`]);
/// - a bare name that the walk finds nowhere is [`Unfound`] for a simulation, whose script starts the program and
///   whose word may not be an executable at all (`app.jar`); the examples, which the runner starts, are refused it.
///
/// What a run of the kind has to say of the words as written is in the settings either way.
///
/// [`InCopy`]: ProgramStart::InCopy
/// [`Host`]: ProgramStart::Host
/// [`Unfound`]: ProgramStart::Unfound
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) enum ProgramStart {
    /// A file outside the copy, with its executable as found now. The copy resolves the word to the same file, or
    /// the run fails as `settings_moved`.
    Host(Executable),
    /// A file of the copy the run makes (it may be written by the build): not digested.
    InCopy,
    /// A bare name of a simulation that `PATH` does not find: not digested, for there is no file to digest.
    Unfound,
}

impl ProgramStart {
    /// Where the `word` of the role `role` starts from, for the project rooted at `root`; the executable of a
    /// [`ProgramStart::Host`] is found, and digested. An error when it cannot be found (but for a bare name of a
    /// simulation) or read; one that is in the copy is neither looked for nor read.
    pub(super) async fn of(
        files: &mut Files,
        root: &Path,
        role: CommandRole,
        word: &str,
    ) -> Result<Self, RunnerError> {
        let search = std::env::var_os("PATH").unwrap_or_default();
        Self::searching(files, root, role, word, &search).await
    }

    /// As [`ProgramStart::of`], with `search` in the place of `PATH`.
    async fn searching(
        files: &mut Files,
        root: &Path,
        role: CommandRole,
        word: &str,
        search: &OsStr,
    ) -> Result<Self, RunnerError> {
        let path = Path::new(word);
        if has_place(word) {
            return Ok(Self::InCopy);
        }
        let file = if path.is_absolute() {
            process::locate_in(root, word, search)
                .await
                .map_err(|f| refused(role.clone(), NotFound::Locate(f)))?
        } else if path.components().count() > 1 {
            return Ok(Self::InCopy);
        } else {
            let found = process::search_path(root, word, search)
                .await
                .map_err(|f| refused(role.clone(), NotFound::Locate(f)))?;
            match found {
                // a relative entry is relative to the copy, which has the file the root has under it
                Some(hit) if hit.relative => return Ok(Self::InCopy),
                Some(hit) => hit.file,
                // the script of a simulation starts the program, and the word may be no executable
                None if role == CommandRole::SimProgram => return Ok(Self::Unfound),
                None => {
                    return Err(RunnerError::Locate {
                        what: role,
                        reason: format!("executable `{word}` not found in PATH"),
                    });
                }
            }
        };
        let digest = files
            .digest(&file)
            .await
            .map_err(|f| refused(role, NotFound::Fingerprint(f)))?;
        Ok(Self::Host(executable_at(root, &file, digest)))
    }

    /// Why the copy `tree` does not resolve the `word` to the file the settings hold, as the run will, when `self`
    /// is a [`ProgramStart::Host`] and that is so: the word is walked from the copy, so a relative entry of `PATH`
    /// finds the copy's own file. None when it resolves to the same file with the same digest, and for the others,
    /// which hold nothing. An error when the file cannot be read.
    async fn moved(
        &self,
        root: &Path,
        tree: &Path,
        word: &str,
        search: &OsStr,
    ) -> Result<Option<String>, process::Failure> {
        let Self::Host(recorded) = self else {
            return Ok(None);
        };
        let was = root.join(&recorded.at);
        let Ok(now) = process::locate_in(tree, word, search).await else {
            return Ok(Some(format!(
                "`{word}` is not found in the copy, and was {} at the project's root, where the settings were digested",
                was.display()
            )));
        };
        if now != was {
            return Ok(Some(format!(
                "`{word}` is {} in the copy and was {} at the project's root, where the settings were digested",
                now.display(),
                was.display()
            )));
        }
        let digest = process::file_digest(&now).await?;
        if digest != recorded.digest {
            return Ok(Some(format!(
                "`{word}` is another file in the copy (sha256 {digest}) than at the project's root (sha256 {}), where the settings were digested",
                recorded.digest
            )));
        }
        Ok(None)
    }
}

/// The program under test of a simulation or of the examples of a Context: where its first word starts it from
/// ([`ProgramStart`]) and its words as written, each a [`Template`] that is made the places of a run when it is
/// started.
///
/// A [`ProgramStart::Host`] file is in the settings digest, and the copy is held to it when it is made
/// ([`ProgramUnderTest::moved`], which `settings_moved` is the failure of). An [`ProgramStart::InCopy`] or
/// [`ProgramStart::Unfound`] one has no digest to hold the copy to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ProgramUnderTest {
    start: ProgramStart,
    program: Template,
    args: Vec<Template>,
}

impl ProgramUnderTest {
    /// The program `words` of `role`, started from where its first word says, for the project rooted at `root`.
    async fn resolve(
        files: &mut Files,
        root: &Path,
        role: CommandRole,
        words: &[Template],
    ) -> Result<Self, RunnerError> {
        let Some((program, args)) = words.split_first() else {
            return Err(no_program(role));
        };
        Ok(Self {
            start: ProgramStart::of(files, root, role, program.as_str()).await?,
            program: program.clone(),
            args: args.to_vec(),
        })
    }

    /// Why the copy `tree` of the project rooted at `root` does not hold the program the settings do, when the
    /// first word starts from the host and the copy, as the run will walk `PATH` from it, resolves it to another
    /// file or to one with another digest (see [`ProgramStart::moved`]); none otherwise.
    pub(super) async fn moved(
        &self,
        root: &Path,
        tree: &Path,
    ) -> Result<Option<String>, process::Failure> {
        let search = std::env::var_os("PATH").unwrap_or_default();
        self.start
            .moved(root, tree, self.program.as_str(), &search)
            .await
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
    async fn resolve(files: &mut Files, config: &Config, root: &Path) -> Result<Self, RunnerError> {
        Ok(Self {
            copy_exclude: config
                .copy_exclude
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            env: config.env.iter().cloned().collect(),
            build: Program::resolve(files, root, CommandRole::Build, &config.build).await?,
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
        files: &mut Files,
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
            copy: CopySettings::resolve(files, config, root).await?,
            test: Program::resolve(files, root, CommandRole::Test, test).await?,
            lang: lang.to_string(),
            provider: Program::resolve(
                files,
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
        files: &mut Files,
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
            copy: CopySettings::resolve(files, config, root).await?,
            interpreter: Program::resolve(files, root, CommandRole::Interpreter, &sim.interpreter)
                .await?,
            program: ProgramUnderTest::resolve(files, root, CommandRole::SimProgram, &sim.program)
                .await?,
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
        files: &mut Files,
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
            copy: CopySettings::resolve(files, config, root).await?,
            program: ProgramUnderTest::resolve(
                files,
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
        let error = Program::resolve(&mut Files::default(), &root, CommandRole::Build, &[])
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
        ProgramStart::searching(
            &mut Files::default(),
            root,
            CommandRole::SimProgram,
            word,
            OsStr::new(search),
        )
        .await
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
    async fn a_word_starts_in_the_copy_when_it_names_a_place_a_relative_path_or_a_relative_entry_of_path()
     {
        let root = dir("start-copy");
        // none of these is looked for or read: the file is the copy's, and may be what the build writes later
        for word in [
            "./target/debug/app",
            "bin/app",
            "{tree}/x",
            "{tree}",
            "{run}/target/debug/app",
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
        // the walk is in order: a relative entry that has it decides before an absolute one that has it too
        let outside = dir("start-copy-outside");
        executable(&outside, "prog", "#!/bin/sh\n");
        assert_eq!(
            start_of(&root, "prog", &format!("bin:{}", outside.display()))
                .await
                .unwrap(),
            ProgramStart::InCopy
        );
        // and the other way round, the absolute entry that comes first decides
        assert!(matches!(
            start_of(&root, "prog", &format!("{}:bin", outside.display()))
                .await
                .unwrap(),
            ProgramStart::Host(_)
        ));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[tokio::test]
    async fn an_entry_of_path_that_cannot_be_read_is_gone_past_and_is_an_error_only_if_the_name_is_found_nowhere()
     {
        use std::os::unix::fs::PermissionsExt;
        let root = dir("start-unreadable");
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // a user who can read it anyway (root) cannot show the error
        if std::fs::read_dir(&locked).is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            let _ = std::fs::remove_dir_all(root);
            return;
        }
        // the name found nowhere, absolute or relative, the entry may have had it: the walk cannot say it does not,
        // so a simulation's word is not `Unfound`
        for search in [locked.to_string_lossy().into_owned(), "locked".to_string()] {
            for role in [CommandRole::SimProgram, CommandRole::ExamplesProgram] {
                let error = ProgramStart::searching(
                    &mut Files::default(),
                    &root,
                    role.clone(),
                    "prog",
                    OsStr::new(&search),
                )
                .await
                .unwrap_err();
                let RunnerError::Locate { what, reason } = &error else {
                    panic!("{error:?} is not a failure to locate");
                };
                assert_eq!(what, &role);
                assert!(
                    reason.contains("locked/prog") && reason.contains("Permission denied"),
                    "{reason}"
                );
            }
        }
        // as `execvp` goes on past an entry it cannot read, a later entry that has the name is the hit, and is the
        // host's or the copy's by that entry
        let outside = dir("start-unreadable-outside");
        executable(&outside, "prog", "#!/bin/sh\n");
        executable(&root, "bin/prog", "#!/bin/sh\n");
        for locked_entry in [locked.to_string_lossy().into_owned(), "locked".to_string()] {
            let search = format!("{locked_entry}:{}", outside.display());
            assert!(
                matches!(
                    start_of(&root, "prog", &search).await.unwrap(),
                    ProgramStart::Host(_)
                ),
                "{search}"
            );
            let search = format!("{locked_entry}:bin");
            assert_eq!(
                start_of(&root, "prog", &search).await.unwrap(),
                ProgramStart::InCopy,
                "{search}"
            );
        }
        let _ = std::fs::remove_dir_all(outside);
        // an entry that has no such file, or is a file and no directory, is only passed over
        std::fs::write(root.join("plain"), "").unwrap();
        for search in ["gone", "plain"] {
            let search = format!("{search}:{}", root.display());
            assert_eq!(
                start_of(&root, "prog", &search).await.unwrap(),
                ProgramStart::Unfound
            );
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn an_absolute_path_is_the_hosts_wherever_it_points_inside_the_project_too() {
        let root = dir("start-absolute");
        let app = executable(&root, "out/app", "#!/bin/sh\n");
        // at run time it is the root's file and not the copy's, so it is held, relative to the root
        let by_path = start_of(&root, &app.to_string_lossy(), "").await.unwrap();
        let ProgramStart::Host(found) = &by_path else {
            panic!("{by_path:?} is not the host's");
        };
        assert_eq!(found.at, "out/app");
        assert_eq!(found.digest, process::file_digest(&app).await.unwrap());
        // and one that is not there is not found, as a host's is not
        let error = start_of(&root, &root.join("out/gone").to_string_lossy(), "")
            .await
            .unwrap_err();
        assert!(matches!(error, RunnerError::Locate { .. }), "{error:?}");
        // a bare name found through an absolute entry of `PATH` that is inside the root is the same file
        let by_name = start_of(&root, "app", &root.join("out").to_string_lossy())
            .await
            .unwrap();
        assert_eq!(by_name, by_path);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_host_program_is_moved_when_the_copy_resolves_it_to_another_file_or_to_other_bytes() {
        let (root, tree) = (dir("moved-root"), dir("moved-tree"));
        let outside = dir("moved-outside");
        let tool = executable(&outside, "tool", "#!/bin/sh\necho one\n");
        let search = format!("bin:{}", outside.display());
        // recorded from the root, where the relative entry `bin` has nothing: the host's
        let start = start_of(&root, "tool", &search).await.unwrap();
        assert!(matches!(&start, ProgramStart::Host(_)), "{start:?}");
        let moved = |start: &ProgramStart| {
            let start = start.clone();
            let (root, tree, search) = (root.clone(), tree.clone(), search.clone());
            async move {
                start
                    .moved(&root, &tree, "tool", OsStr::new(&search))
                    .await
                    .unwrap()
            }
        };
        // the copy resolves it to the same file with the same bytes: not moved
        assert_eq!(moved(&start).await, None);
        // the copy has the name under the relative entry, which comes first in the walk from the copy
        executable(&tree, "bin/tool", "#!/bin/sh\necho one\n");
        let why = moved(&start).await.expect("moved");
        assert!(
            why.contains(&format!("{}", tree.join("bin/tool").display()))
                && why.contains(&format!("{}", tool.display())),
            "{why}"
        );
        std::fs::remove_dir_all(tree.join("bin")).unwrap();
        // the file is other bytes now
        executable(&outside, "tool", "#!/bin/sh\necho two\n");
        let why = moved(&start).await.expect("moved");
        assert!(why.contains("another file in the copy (sha256 "), "{why}");
        // gone from where the copy looks
        std::fs::remove_file(&tool).unwrap();
        let why = moved(&start).await.expect("moved");
        assert!(why.contains("is not found in the copy"), "{why}");
        // a program of the copy, or one nothing found, holds nothing to be moved from
        for start in [ProgramStart::InCopy, ProgramStart::Unfound] {
            assert_eq!(moved(&start).await, None);
        }
        // and the program under test asks it of its first word, from the environment's `PATH`
        let sh = ProgramUnderTest::resolve(
            &mut Files::default(),
            &root,
            CommandRole::ExamplesProgram,
            &[serde_json::from_value("sh".into()).unwrap()],
        )
        .await
        .unwrap();
        assert!(matches!(sh.start, ProgramStart::Host(_)));
        assert_eq!(sh.moved(&root, &tree).await.unwrap(), None);
        for dir in [root, tree, outside] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn a_host_program_that_is_not_there_or_not_a_file_says_whose_it_is() {
        let root = dir("start-missing");
        // a bare name that `PATH` finds nowhere: the script of a simulation starts the program, and the word may be
        // no executable (`app.jar`), so it is not digested and not an error; the runner starts the examples' program
        assert_eq!(
            start_of(&root, "chidev-no-such-program", "").await.unwrap(),
            ProgramStart::Unfound
        );
        let error = ProgramStart::searching(
            &mut Files::default(),
            &root,
            CommandRole::ExamplesProgram,
            "chidev-no-such-program",
            OsStr::new(""),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "the program of the examples: executable `chidev-no-such-program` not found in PATH"
        );
        assert!(matches!(error, RunnerError::Locate { .. }));
        // an absolute path that is not there is not found, for either
        let error = ProgramStart::searching(
            &mut Files::default(),
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
        // a name that `PATH` finds nowhere, with a relative entry that has no such file, is the same for a simulation
        assert_eq!(
            start_of(&root, "prog", "bin").await.unwrap(),
            ProgramStart::Unfound
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[tokio::test]
    async fn a_program_under_test_holds_its_words_as_written_and_where_the_first_starts_from() {
        let root = dir("under-test");
        let templates =
            |list: &[&str]| -> Vec<Template> { serde_json::from_value(list.into()).unwrap() };
        let under = ProgramUnderTest::resolve(
            &mut Files::default(),
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
        let error =
            ProgramUnderTest::resolve(&mut Files::default(), &root, CommandRole::SimProgram, &[])
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
        let inside = Program::resolve(
            &mut Files::default(),
            &root,
            CommandRole::Test,
            &words(&["./tool.sh", "-x"]),
        )
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
        let outside = Program::resolve(
            &mut Files::default(),
            &root,
            CommandRole::Test,
            &words(&["sh"]),
        )
        .await
        .unwrap();
        assert!(Path::new(&outside.executable().at).is_absolute());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(copy);
    }

    #[tokio::test]
    async fn a_file_is_read_once_however_many_commands_start_it_and_each_command_keeps_its_own_words()
     {
        let root = dir("files-shared");
        let tool = executable(&root, "tool.sh", "#!/bin/sh\n");
        let mut files = Files::default();
        // one file, started by two commands with other arguments, and spelled three ways
        let first = Program::resolve(
            &mut files,
            &root,
            CommandRole::Test,
            &words(&["./tool.sh", "-a"]),
        )
        .await
        .unwrap();
        let second = Program::resolve(
            &mut files,
            &root,
            CommandRole::Build,
            &words(&["./tool.sh", "-b"]),
        )
        .await
        .unwrap();
        let absolute = tool.to_string_lossy().into_owned();
        let by_path = Program::resolve(
            &mut files,
            &root,
            CommandRole::Interpreter,
            &words(&[&absolute, "-a"]),
        )
        .await
        .unwrap();
        assert_eq!(files.read(), 1);
        // the file is the same and the commands are not: their digests differ by their arguments
        assert_eq!(first.executable(), second.executable());
        assert_ne!(first, second);
        assert_ne!(digest_of(&first).unwrap(), digest_of(&second).unwrap());
        assert_eq!(first.executable(), by_path.executable());
        // a file of the host that is a program under test is the same file, and is not read again either
        let start = ProgramStart::searching(
            &mut files,
            &root,
            CommandRole::ExamplesProgram,
            &absolute,
            OsStr::new(""),
        )
        .await
        .unwrap();
        let ProgramStart::Host(host) = &start else {
            panic!("{start:?} is not the host's");
        };
        assert_eq!(host, first.executable());
        // a program of the copy has no digest and is not read
        let in_copy = ProgramStart::searching(
            &mut files,
            &root,
            CommandRole::ExamplesProgram,
            "./target/debug/app",
            OsStr::new(""),
        )
        .await
        .unwrap();
        assert_eq!(in_copy, ProgramStart::InCopy);
        assert_eq!(files.read(), 1);
        // the memo is the value's: the file changed, and what this one has is what was read when it asked first
        executable(&root, "tool.sh", "#!/bin/sh\n# changed\n");
        let stale = Program::resolve(&mut files, &root, CommandRole::Test, &words(&["./tool.sh"]))
            .await
            .unwrap();
        assert_eq!(stale.executable(), first.executable());
        let mut fresh = Files::default();
        let now = Program::resolve(&mut fresh, &root, CommandRole::Test, &words(&["./tool.sh"]))
            .await
            .unwrap();
        assert_ne!(now.executable(), first.executable());
        assert_eq!(fresh.read(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_read_is_not_read_again_and_says_so_each_time() {
        let root = dir("files-unreadable");
        std::fs::create_dir_all(root.join("adir")).unwrap();
        let mut files = Files::default();
        for role in [CommandRole::Build, CommandRole::Test] {
            let error = Program::resolve(&mut files, &root, role.clone(), &words(&["./adir"]))
                .await
                .unwrap_err();
            assert!(
                matches!(&error, RunnerError::Fingerprint { what, .. } if *what == role),
                "{error:?}"
            );
        }
        assert_eq!(files.read(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
