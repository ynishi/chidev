//! The runner of measures. It copies the project into a directory of its own *under the project*
//! (`.chidev/measure/<run>/`), writes the code the Pin accepted and the Harness sources of the model over the copy,
//! builds and tests the copy once as it is (the baseline), asks a provider for mutants, and for each mutant applies
//! the edit, builds (a build that fails is *stillborn*), tests (a failure *kills* the mutant, a pass lets it
//! *survive*, a deadline is a *timeout*) and puts the code back. The copy has a target directory of its own, and
//! the runner removes its directory when it is done, so the project's files and build output are never touched.
//!
//! Which commands to run and where each Interface and Harness lives is the project's `[measure]` table of
//! `chidev.toml` ([`load`]). Every command is a program started without a shell, in a process group of its own,
//! under a deadline ([`crate::infra::process`]); the whole run has a time budget.
//!
//! A run does not read the table as written. It first resolves what its kind reads of it into the settings of that
//! kind (`settings.rs`: defaults applied, programs found, their executables digested), and runs from those and from
//! nothing else; the digest of the same settings is the settings digest the measurement is stamped with.
//!
//! A run that cannot measure (the baseline fails, a command or the provider fails, the budget is used up) still
//! reports what it observed: the failure, the commands' evidence and the mutants it got through. Only a
//! configuration that cannot measure this Interface at all is an error, and then nothing was run.
//!
//! A simulation (`sim.rs`, which holds the protocol `chidev-sim 2`) is run in the same kind of copy: the pinned code
//! of the Interfaces the sim Harness builds and the script are written over it, the build command runs, and the
//! script is started once per case under the protocol `chidev-sim 2`. The sampled cases are then run a second time
//! in a second, fresh copy.
//!
//! The examples of a Harness of examples (`examples.rs`) are run in the same kind of copy, prepared the same way: the
//! pinned code of the Interfaces the Harness builds is written over it and the build command runs. The program is then
//! started once per example, with the example's arguments and input, and the answer is judged by the same judge as
//! `chidev accept` uses.
//!
//! The same configuration answers a second question for the use cases, as the [`ItemSource`] port: the language of an
//! Interface (its layout's `lang`) and the Items a provider of that language finds in its code (`items.rs`, which
//! holds the protocol `chidev-items 1`). That provider is started in the project's root, not in a copy, under the
//! `timeout` of the table.

mod config;
mod examples;
mod items;
mod provider;
mod settings;
mod sim;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use thiserror::Error;

use self::config::{Config, RunDirs, WORK_DIR};
use self::settings::{
    CopySettings, ExamplesSettings, Files, MutationSettings, NotFound, Program, ProgramUnderTest,
    SimSettings, find,
};
use super::process::{self, Capture, Over, Pipe};
use super::project::{Project, Section};
use crate::app::ports::items::{FoundItem, ItemSource, ItemsError};
use crate::app::ports::measure::{
    BoxFuture, ExamplesReport, ExamplesRequest, LayoutPlace, MutateReport, MutateRequest, Runner,
    RunnerError, SettingsSaid, SettingsTarget, SimulateReport, SimulateRequest,
};
use crate::domain::vo::measure::{
    FailureKind, MeasureFailure, MutantOutcome, MutantResult, SettingsDigest, StepEvidence,
};
use crate::domain::vo::name::{AssetName, ContextName};

/// What a command may write to standard output and standard error before the rest is thrown away.
const OUTPUT_LIMIT: usize = 256 * 1024;
const ERROR_LIMIT: usize = 64 * 1024;

/// How much of a command's output goes into the message of a failure and into its evidence.
const TAIL: usize = 1200;

/// Why the `[measure]` table could not be loaded. A malformed table is not silently skipped.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}: {message}", path.display())]
pub struct MeasureConfigError {
    /// The configuration file.
    pub path: PathBuf,
    /// The read or validation error.
    pub message: String,
}

/// The process implementation of [`Runner`], and of [`ItemSource`] over the same table.
#[derive(Debug, Clone)]
pub struct ProcessRunner {
    root: PathBuf,
    config: Config,
}

/// Loads the `[measure]` table of the project's settings. A project without the table has no runner (`None`); a
/// table that is wrong is an error naming the field.
pub fn load(project: &Project) -> Result<Option<ProcessRunner>, MeasureConfigError> {
    let Some(measure) = project.section(Section::Measure) else {
        return Ok(None);
    };
    let config = config::parse(measure).map_err(|m| MeasureConfigError {
        path: project.file().to_path_buf(),
        message: format!("measure: {m}"),
    })?;
    Ok(Some(ProcessRunner {
        root: project.root().to_path_buf(),
        config,
    }))
}

impl Runner for ProcessRunner {
    /// Resolves every target by the one path the single asks take, with one memo of the files read for the whole
    /// call ([`Files`]): a program that several targets use is found and hashed once, and each command is still its
    /// own [`Program`], so two commands that start one binary with other arguments keep their own digests. The memo
    /// is dropped when the call ends; nothing is kept between calls. Only the files of build, test, provider,
    /// interpreter and of a `ProgramStart::Host` program under test are in it: a program that starts in the copy,
    /// or that `PATH` does not find, has no digest.
    fn settings_of<'a>(&'a self, targets: &'a [SettingsTarget]) -> BoxFuture<'a, SettingsSaid> {
        Box::pin(self.digests_of(targets))
    }

    fn settings<'a>(
        &'a self,
        context: &'a ContextName,
        interface: &'a AssetName,
        harnesses: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async move {
            self.digest_of_one(&SettingsTarget::Mutation {
                context: context.clone(),
                interface: interface.clone(),
                harnesses: harnesses.to_vec(),
            })
            .await
        })
    }

    fn mutate<'a>(
        &'a self,
        request: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(self.mutate_run(request))
    }

    fn sim_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async move {
            self.digest_of_one(&SettingsTarget::Simulation {
                context: context.clone(),
                harness: harness.clone(),
                builds: builds.to_vec(),
            })
            .await
        })
    }

    fn simulate<'a>(
        &'a self,
        request: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        Box::pin(self.simulate_run(request))
    }

    fn examples_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async move {
            self.digest_of_one(&SettingsTarget::Examples {
                context: context.clone(),
                harness: harness.clone(),
                builds: builds.to_vec(),
            })
            .await
        })
    }

    fn run_examples<'a>(
        &'a self,
        request: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>> {
        Box::pin(self.examples_run(request))
    }
}

impl ItemSource for ProcessRunner {
    fn language(&self, context: &ContextName, interface: &AssetName) -> Result<String, ItemsError> {
        match self.config.interface(context, interface) {
            Some((_, Some(lang))) => Ok(lang.to_string()),
            _ => Err(ItemsError::NoLanguage),
        }
    }

    fn items<'a>(
        &'a self,
        lang: &'a str,
        source: &'a str,
    ) -> BoxFuture<'a, Result<Vec<FoundItem>, ItemsError>> {
        Box::pin(self.items_of(lang, source))
    }
}

/// A failure of a run, from one of the process module's.
fn from_process(f: process::Failure, over_budget: bool) -> MeasureFailure {
    match f.kind {
        process::FailureKind::Timeout if over_budget => {
            MeasureFailure::of(FailureKind::Budget, "the time budget of the run is used up")
        }
        process::FailureKind::Timeout => {
            MeasureFailure::of(FailureKind::Timeout, "a command did not end in time")
        }
        _ => MeasureFailure::of(FailureKind::Io, format!("{}: {}", f.kind.word(), f.message)),
    }
}

fn io(what: &str, e: impl std::fmt::Display) -> MeasureFailure {
    MeasureFailure::of(FailureKind::Io, format!("{what}: {e}"))
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl ProcessRunner {
    /// The digest of the settings of `target`, its programs found through `files`. The one place a settings struct
    /// is resolved for a digest, for the single asks and for the batch.
    async fn digest_of_target(
        &self,
        files: &mut Files,
        target: &SettingsTarget,
    ) -> Result<SettingsDigest, RunnerError> {
        match target {
            SettingsTarget::Mutation {
                context,
                interface,
                harnesses,
            } => MutationSettings::resolve(
                files,
                &self.config,
                &self.root,
                context,
                interface,
                harnesses,
            )
            .await?
            .digest(),
            SettingsTarget::Simulation {
                context,
                harness,
                builds,
            } => SimSettings::resolve(files, &self.config, &self.root, context, harness, builds)
                .await?
                .digest(),
            // The Harness is named by the port but the run reads nothing of it: the examples are the request's.
            SettingsTarget::Examples {
                context, builds, ..
            } => ExamplesSettings::resolve(files, &self.config, &self.root, context, builds)
                .await?
                .digest(),
        }
    }

    /// The digest of the settings of the one `target`, with a memo of its own.
    async fn digest_of_one(&self, target: &SettingsTarget) -> Result<SettingsDigest, RunnerError> {
        self.digest_of_target(&mut Files::default(), target).await
    }

    /// The digest of the settings of each of `targets`, each beside its target, all through one memo of the files
    /// read ([`Files`]) that ends with the call.
    async fn digests_of(&self, targets: &[SettingsTarget]) -> SettingsSaid {
        let mut files = Files::default();
        let mut said = Vec::with_capacity(targets.len());
        for target in targets {
            let digest = self.digest_of_target(&mut files, target).await;
            said.push((target.clone(), digest));
        }
        said
    }

    /// The Items the provider of `lang` finds in `source`.
    async fn items_of(&self, lang: &str, source: &str) -> Result<Vec<FoundItem>, ItemsError> {
        let command = self
            .config
            .items
            .get(lang)
            .map(Vec::as_slice)
            .filter(|c| !c.is_empty())
            .ok_or(ItemsError::NoProvider)?;
        let deadline =
            process::deadline_in(self.config.timeout).map_err(|f| ItemsError::Failed(f.message))?;
        items::ask(&self.root, command, source, deadline).await
    }

    /// The places of the run `run`, under the project's [`WORK_DIR`]; a name that is not letters, digits and `-` is
    /// refused, so a run never reaches outside its directory.
    fn run_dirs(&self, run: &str) -> Result<RunDirs, RunnerError> {
        let safe = !run.is_empty() && run.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !safe {
            return Err(RunnerError::BadRunName(run.to_string()));
        }
        Ok(RunDirs::of(self.root.join(WORK_DIR).join(run)))
    }

    async fn mutate_run(&self, request: &MutateRequest) -> Result<MutateReport, RunnerError> {
        let names: Vec<AssetName> = request.harnesses.iter().map(|h| h.name.clone()).collect();
        let settings = MutationSettings::resolve(
            &mut Files::default(),
            &self.config,
            &self.root,
            &request.context,
            &request.interface,
            &names,
        )
        .await?;
        let digest = settings.digest()?;
        let mut harnesses = Vec::with_capacity(request.harnesses.len());
        for h in &request.harnesses {
            let path = settings
                .harness_path(&h.name)
                .ok_or_else(|| RunnerError::NoLayout {
                    place: LayoutPlace::Harness,
                    context: request.context.clone(),
                    name: h.name.clone(),
                })?;
            harnesses.push((path, h.source.as_str()));
        }
        let dirs = self.run_dirs(&request.run)?;
        let job = Job {
            root: &self.root,
            copy: &settings.copy,
            dirs,
            deadline: process::deadline_in(settings.budget)
                .map_err(|f| RunnerError::Deadline(f.message))?,
        };
        let mut progress = Progress::default();
        let finished = job
            .steps(
                &settings,
                (settings.interface.path(), &request.code),
                &harnesses,
                &mut progress,
            )
            .await;
        job.clean().await;
        Ok(MutateReport {
            settings: digest,
            sites: progress.sites,
            planned: progress.planned,
            mutants: progress.mutants,
            failure: finished.err(),
            evidence: progress.evidence,
        })
    }
}

/// What a run has observed so far.
#[derive(Debug, Default)]
struct Progress {
    sites: u32,
    planned: u32,
    mutants: Vec<MutantResult>,
    evidence: Vec<StepEvidence>,
}

/// A command found, with the digest of its executable.
struct Command {
    executable: PathBuf,
    args: Vec<String>,
    digest: String,
}

impl Command {
    fn line(&self) -> String {
        std::iter::once(self.executable.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// What a command did.
struct Ran {
    success: bool,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// One run in its directory. It holds the project's root and the part of the settings every kind of run shares, and
/// nothing of the table or of the runner: what else a run reads is handed to the steps that read it, from the
/// settings of its kind ([`MutationSettings`], [`SimSettings`]).
struct Job<'a> {
    /// The project's root: what is copied, and where the provider is started.
    root: &'a Path,
    /// How the project is copied and built, and the environment the commands run in.
    copy: &'a CopySettings,
    dirs: RunDirs,
    deadline: tokio::time::Instant,
}

impl Job<'_> {
    fn over_budget(&self) -> bool {
        tokio::time::Instant::now() >= self.deadline
    }

    fn fail(&self, f: process::Failure) -> MeasureFailure {
        from_process(f, self.over_budget())
    }

    /// The environment of the commands, with `{run}` made the run's directory and `{tree}` the root of the copy.
    fn env(&self) -> Vec<(String, String)> {
        self.copy
            .env
            .iter()
            .map(|(k, v)| (k.clone(), self.dirs.expand(v)))
            .collect()
    }

    /// Checks that `program` is, in the copy as it is made, the executable the settings were digested with. A copy
    /// where the same name is another file (the code or a Harness written over it, say) would be a run under other
    /// settings than the ones it is stamped with. It is asked once, after the copy is made and before any command
    /// runs: what a command (the build) writes afterwards is part of the run and not a change of the settings. A
    /// program the copy does not have yet is not asked about; [`Job::command`] stops the run if it is still not
    /// there when it is started.
    async fn verify(&self, program: &Program) -> Result<(), MeasureFailure> {
        let found = match find(&self.dirs.tree, program.program()).await {
            Ok(found) => found,
            Err(NotFound::Locate(_)) => return Ok(()),
            Err(e) => return Err(self.fail(e.failure())),
        };
        let expected = program.executable().digest();
        if found.digest != expected {
            return Err(MeasureFailure::of(
                FailureKind::SettingsMoved,
                format!(
                    "`{}` is another file in the copy (sha256 {}) than at the project's root (sha256 {}), where the settings were digested",
                    program.program(),
                    found.digest,
                    expected
                ),
            ));
        }
        Ok(())
    }

    /// Checks that the program under test `program`, when its first word starts from the host, is resolved from the
    /// copy as the run will resolve it (a relative entry of `PATH` is the copy's) to the file the settings were
    /// digested with. Asked at the same moment as [`Job::verify`]. A program in the copy has no digest to be held to.
    async fn verify_under_test(&self, program: &ProgramUnderTest) -> Result<(), MeasureFailure> {
        match program.moved(self.root, &self.dirs.tree).await {
            Ok(None) => Ok(()),
            Ok(Some(why)) => Err(MeasureFailure::of(FailureKind::SettingsMoved, why)),
            Err(f) => Err(self.fail(f)),
        }
    }

    /// The command `program` in the copy: the file it names there now, which is the one the run starts. Its digest
    /// is the evidence of what ran.
    async fn command(&self, program: &Program) -> Result<Command, MeasureFailure> {
        let found = find(&self.dirs.tree, program.program())
            .await
            .map_err(|e| self.fail(e.failure()))?;
        Ok(Command {
            executable: found.executable,
            args: program.args().to_vec(),
            digest: found.digest,
        })
    }

    /// Makes the copy a program is run from: copies the project, writes `written` over it (the code the Pins
    /// accepted, and any file the kind of run owns, each at its path in the copy), checks that the build command and
    /// the `programs` of the run are in the copy the files the settings were digested with (as is the program under
    /// test when it starts from the host, [`Job::verify_under_test`]), and runs the build command
    /// under the time of the baseline. `tag` names the step in the evidence (`<tag>-build`), which is put in
    /// `evidence` whether or not the build passes. A build that fails is the failure `no_build`: there is no program
    /// to run. The one preparation of every run that builds a program from pinned code, so that a simulation and a
    /// run of examples cannot be prepared differently.
    async fn prepare(
        &self,
        tag: &str,
        written: &[(&Path, &str)],
        programs: &[&Program],
        under_test: &ProgramUnderTest,
        evidence: &mut Vec<StepEvidence>,
    ) -> Result<(), MeasureFailure> {
        let (root, tree, exclude) = (
            self.root.to_path_buf(),
            self.dirs.tree.clone(),
            self.copy.excluded(),
        );
        tokio::task::spawn_blocking(move || copy_tree(&root, &tree, &exclude))
            .await
            .map_err(|e| io("the copy of the project", e))?
            .map_err(|e| io("cannot copy the project", e))?;
        for (path, text) in written {
            self.write(path, text).await?;
        }
        // Every program of the copy is the one the settings were digested with, as the copy is made. The build runs
        // after that and may write an executable (the interpreter, say): that is the run's, not a moved setting.
        self.verify(&self.copy.build).await?;
        for program in programs {
            self.verify(program).await?;
        }
        // The program under test is held to the settings only when its first word starts from the host.
        self.verify_under_test(under_test).await?;
        let env = self.env();
        let build = self.command(&self.copy.build).await?;
        let started = Instant::now();
        let result = self.exec(&build, &env, self.copy.baseline_timeout).await;
        let (exit_code, stderr) = match &result {
            Ok((ran, _)) => (ran.exit_code, provider::tail(&ran.stderr, TAIL)),
            Err(_) => (None, String::new()),
        };
        evidence.push(StepEvidence {
            step: format!("{tag}-build"),
            command: build.line(),
            command_digest: Some(build.digest.clone()),
            exit_code,
            duration_ms: millis(started),
            stderr,
        });
        let (ran, _) = result.map_err(|f| self.fail(f))?;
        if !ran.success {
            let said = [
                provider::tail(&ran.stderr, TAIL / 2),
                provider::tail(&ran.stdout, TAIL / 2),
            ]
            .join("\n");
            return Err(MeasureFailure::of(
                FailureKind::NoBuild,
                format!(
                    "the program does not build (exit {}): {}",
                    ran.exit_code
                        .map_or_else(|| "by a signal".to_string(), |c| c.to_string()),
                    said.trim()
                ),
            ));
        }
        Ok(())
    }

    /// Runs `command` in the copy for at most `limit`, and not past the budget.
    async fn exec(
        &self,
        command: &Command,
        env: &[(String, String)],
        limit: std::time::Duration,
    ) -> Result<(Ran, Capture), process::Failure> {
        let deadline = process::deadline_in(limit)?.min(self.deadline);
        let mut capture = Capture::default();
        let status = process::run(
            &process::Spec {
                executable: &command.executable,
                args: &command.args,
                cwd: &self.dirs.tree,
                env,
                stdin: None,
                deadline,
                stdout: Pipe {
                    limit: OUTPUT_LIMIT,
                    over: Over::Truncate,
                },
                stderr: Pipe {
                    limit: ERROR_LIMIT,
                    over: Over::Truncate,
                },
            },
            &mut capture,
        )
        .await?;
        let ran = Ran {
            success: status.success(),
            exit_code: capture.exit_code,
            stdout: std::mem::take(&mut capture.stdout),
            stderr: std::mem::take(&mut capture.stderr),
        };
        Ok((ran, capture))
    }

    /// The build or test before any change: it has to pass, or there is nothing to measure against.
    async fn baseline(
        &self,
        step: &'static str,
        command: &Command,
        env: &[(String, String)],
        out: &mut Progress,
    ) -> Result<(), MeasureFailure> {
        let started = Instant::now();
        let result = self.exec(command, env, self.copy.baseline_timeout).await;
        let (exit_code, stderr) = match &result {
            Ok((ran, _)) => (ran.exit_code, provider::tail(&ran.stderr, TAIL)),
            Err(_) => (None, String::new()),
        };
        out.evidence.push(StepEvidence {
            step: step.to_string(),
            command: command.line(),
            command_digest: Some(command.digest.clone()),
            exit_code,
            duration_ms: millis(started),
            stderr,
        });
        let (ran, _) = result.map_err(|f| self.fail(f))?;
        if ran.success {
            return Ok(());
        }
        let said = [
            provider::tail(&ran.stderr, TAIL / 2),
            provider::tail(&ran.stdout, TAIL / 2),
        ]
        .join("\n");
        Err(MeasureFailure::of(
            FailureKind::Baseline,
            format!(
                "the {step} failed before any change (exit {}): {}",
                ran.exit_code
                    .map_or_else(|| "by a signal".to_string(), |c| c.to_string()),
                said.trim()
            ),
        ))
    }

    /// What became of the code in the copy as it is now: built, then tested, each for at most `timeout`.
    async fn judge(
        &self,
        build: &Command,
        test: &Command,
        env: &[(String, String)],
        timeout: std::time::Duration,
    ) -> Result<MutantOutcome, MeasureFailure> {
        match self.exec(build, env, timeout).await {
            Ok((ran, _)) if !ran.success => return Ok(MutantOutcome::Stillborn),
            Ok(_) => {}
            Err(f) if f.kind == process::FailureKind::Timeout && !self.over_budget() => {
                return Ok(MutantOutcome::Timeout);
            }
            Err(f) => return Err(self.fail(f)),
        }
        match self.exec(test, env, timeout).await {
            Ok((ran, _)) if ran.success => Ok(MutantOutcome::Survived),
            Ok(_) => Ok(MutantOutcome::Killed),
            Err(f) if f.kind == process::FailureKind::Timeout && !self.over_budget() => {
                Ok(MutantOutcome::Timeout)
            }
            Err(f) => Err(self.fail(f)),
        }
    }

    async fn write(&self, path: &Path, text: &str) -> Result<(), MeasureFailure> {
        let target = self.dirs.tree.join(path);
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| io(&format!("cannot make {}", dir.display()), e))?;
        }
        tokio::fs::write(&target, text)
            .await
            .map_err(|e| io(&format!("cannot write {}", target.display()), e))
    }

    async fn steps(
        &self,
        settings: &MutationSettings,
        code: (&Path, &str),
        harnesses: &[(&Path, &str)],
        out: &mut Progress,
    ) -> Result<(), MeasureFailure> {
        let (root, tree, exclude) = (
            self.root.to_path_buf(),
            self.dirs.tree.clone(),
            self.copy.excluded(),
        );
        tokio::task::spawn_blocking(move || copy_tree(&root, &tree, &exclude))
            .await
            .map_err(|e| io("the copy of the project", e))?
            .map_err(|e| io("cannot copy the project", e))?;
        let (path, code) = code;
        self.write(path, code).await?;
        for (path, source) in harnesses {
            self.write(path, source).await?;
        }
        self.verify(&self.copy.build).await?;
        self.verify(&settings.test).await?;
        let env = self.env();
        let build = self.command(&self.copy.build).await?;
        let test = self.command(&settings.test).await?;
        self.baseline("baseline-build", &build, &env, out).await?;
        self.baseline("baseline-test", &test, &env, out).await?;

        let (asked, evidence) = provider::ask(
            self.root,
            &settings.provider.words(),
            &path.to_string_lossy(),
            code,
            settings.max_mutants,
            settings.seed,
            settings.operators.as_deref(),
            self.deadline,
        )
        .await;
        // The provider is started at the project's root, not in the copy, and after the baseline, which can take
        // long: the file it started has to be the one the settings were digested with, or the run would be kept
        // under a stamp that names another provider.
        let ran = evidence.command_digest.clone();
        out.evidence.push(evidence);
        let expected = settings.provider.executable().digest();
        if let Some(ran) = ran.filter(|ran| ran != expected) {
            return Err(MeasureFailure::of(
                FailureKind::SettingsMoved,
                format!(
                    "`{}` is another file at the project's root (sha256 {ran}) than when the settings were digested (sha256 {expected})",
                    settings.provider.program(),
                ),
            ));
        }
        let mutants = asked?;
        out.sites = mutants.sites;
        // What the provider gave is what the run owes; the sites it did not give (the cap) are told apart from the
        // mutants a run did not get to (a failure).
        out.planned = u32::try_from(mutants.planned.len()).unwrap_or(u32::MAX);
        for planned in &mutants.planned {
            if self.over_budget() {
                return Err(from_process(
                    process::Failure {
                        kind: process::FailureKind::Timeout,
                        message: String::new(),
                    },
                    true,
                ));
            }
            let (start, end) = planned.span;
            let edited = format!("{}{}{}", &code[..start], planned.mutant.to(), &code[end..]);
            self.write(path, &edited).await?;
            let outcome = self.judge(&build, &test, &env, settings.timeout).await;
            // The code goes back whatever became of the mutant, so the next one is a change of the original.
            self.write(path, code).await?;
            out.mutants.push(MutantResult {
                mutant: planned.mutant.clone(),
                outcome: outcome?,
            });
        }
        Ok(())
    }

    /// Removes the run's directory. Failing to is not the run's failure: the measurement was made.
    async fn clean(&self) {
        let dir = self.dirs.dir.clone();
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(dir)).await;
    }
}

/// Copies the project at `from` into `to`, leaving out `exclude` (paths relative to `from`) and the directory runs
/// work in. Symbolic links are not followed or copied: a link could lead a write out of the copy.
fn copy_tree(from: &Path, to: &Path, exclude: &[PathBuf]) -> std::io::Result<()> {
    let skip: BTreeSet<PathBuf> = exclude
        .iter()
        .cloned()
        .chain(std::iter::once(PathBuf::from(WORK_DIR)))
        .collect();
    std::fs::create_dir_all(to)?;
    fn walk(dir: &Path, rel: &Path, to: &Path, skip: &BTreeSet<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let rel = rel.join(entry.file_name());
            if skip.contains(&rel) {
                continue;
            }
            let kind = entry.file_type()?;
            let target = to.join(&rel);
            if kind.is_dir() {
                std::fs::create_dir_all(&target)?;
                walk(&entry.path(), &rel, to, skip)?;
            } else if kind.is_file() {
                std::fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    }
    walk(from, Path::new(""), to, &skip)
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::app::ports::measure::{CommandRole, HarnessText};
    use crate::infra::project::{self, FILE};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// Loads the runner of the project rooted at `dir`.
    fn load(dir: &Path) -> Result<Option<ProcessRunner>, MeasureConfigError> {
        super::load(&project::find(dir).unwrap())
    }

    /// The code the Pin accepted; the working tree holds other text in its place.
    const PINNED: &str = "a <= 5 and b + 1\n";

    /// A provider that ignores its input and answers three mutants of [`PINNED`]: `<=` to `<`, `+` to a word that
    /// does not build, and `5` to `6`.
    const PROVIDER: &str = r#"#!/bin/sh
cat > /dev/null
touch provider-called
printf '%s' '{"protocol":1,"sites":3,"mutants":[
{"key":"f/rel/1/0000","line":1,"col":3,"operator":"rel","span":[2,4],"from":"<=","to":"<"},
{"key":"f/arith/1/0000","line":1,"col":14,"operator":"arith","span":[13,14],"from":"+","to":"BROKEN"},
{"key":"f/int/1/0000","line":1,"col":6,"operator":"int","span":[5,6],"from":"5","to":"6"}]}'
"#;

    struct Project(PathBuf);

    impl Project {
        fn new(table: &str) -> Project {
            let dir = std::env::temp_dir().join(format!(
                "chidev-runner-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(dir.join("target")).unwrap();
            std::fs::write(dir.join("code.txt"), "the working tree's own text\n").unwrap();
            std::fs::write(dir.join("target/artifact"), "build output").unwrap();
            std::fs::write(dir.join("provider.sh"), PROVIDER).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    dir.join("provider.sh"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            let project = Project(dir);
            project.table(table);
            project
        }

        /// Writes `table` (the keys of `[measure]`, its sub-tables written bare) as the project's `chidev.toml`.
        fn table(&self, table: &str) {
            let text = table
                .replace("[mutants]", "[measure.mutants]")
                .replace("[items]", "[measure.items]")
                .replace("[[layout]]", "[[measure.layout]]");
            std::fs::write(self.0.join(FILE), format!("[measure]\n{text}")).unwrap();
        }

        fn runner(&self) -> ProcessRunner {
            load(&self.0).unwrap().expect("a [measure] table")
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A project whose build refuses text with BROKEN and whose test wants `a <= ` and the Harness's text.
    const TABLE: &str = r#"
build = ["sh", "-c", "! grep -q BROKEN code.txt"]
test = ["sh", "-c", "grep -q 'a <= ' code.txt && grep -q HARNESS check.txt"]
timeout = 5
env = {}
[mutants]
text = ["./provider.sh"]
[[layout]]
context = "ctx"
interface = "api"
lang = "text"
path = "code.txt"
[[layout]]
context = "ctx"
harness = "checks"
path = "check.txt"
"#;

    fn context(name: &str) -> ContextName {
        ContextName::try_from(name).unwrap()
    }

    fn asset(name: &str) -> AssetName {
        AssetName::try_from(name).unwrap()
    }

    fn request(run: &str) -> MutateRequest {
        MutateRequest {
            run: run.to_string(),
            context: context("ctx"),
            interface: asset("api"),
            code: PINNED.to_string(),
            harnesses: vec![HarnessText {
                name: asset("checks"),
                source: "HARNESS\n".to_string(),
            }],
        }
    }

    fn outcomes(report: &MutateReport) -> Vec<(&str, &'static str)> {
        report
            .mutants
            .iter()
            .map(|m| (m.mutant.key().as_str(), m.outcome.word()))
            .collect()
    }

    #[tokio::test]
    async fn the_pinned_code_is_run_each_mutant_has_its_outcome_and_the_project_is_untouched() {
        let project = Project::new(TABLE);
        let report = project
            .runner()
            .mutate_run(&request("run-1"))
            .await
            .unwrap();
        assert_eq!(report.failure, None, "{report:#?}");
        assert_eq!(
            outcomes(&report),
            [
                ("f/rel/1/0000", "killed"),
                ("f/arith/1/0000", "stillborn"),
                ("f/int/1/0000", "survived"),
            ]
        );
        assert_eq!((report.sites, report.planned), (3, 3));
        // the commands that ran are on record, with the digest of the program
        let steps: Vec<&str> = report.evidence.iter().map(|e| e.step.as_str()).collect();
        assert_eq!(steps, ["baseline-build", "baseline-test", "provider"]);
        assert!(report.evidence.iter().all(|e| e.command_digest.is_some()));
        assert!(report.evidence.iter().all(|e| e.exit_code == Some(0)));
        // the project's own files were not changed, the copy is gone, and the build output was not copied
        assert_eq!(
            std::fs::read_to_string(project.0.join("code.txt")).unwrap(),
            "the working tree's own text\n"
        );
        assert!(!project.0.join("check.txt").exists());
        assert!(!project.0.join(WORK_DIR).join("run-1").exists());
        // the provider runs in the project's root, where it is configured, and not in the copy
        assert!(project.0.join("provider-called").exists());
    }

    #[tokio::test]
    async fn sites_the_provider_found_and_did_not_give_are_told_from_mutants_given() {
        let project = Project::new(TABLE);
        // the provider saw 5 sites and, as `max` asks of it, gave 3 mutants
        std::fs::write(
            project.0.join("provider.sh"),
            PROVIDER.replace("\"sites\":3", "\"sites\":5"),
        )
        .unwrap();
        let report = project
            .runner()
            .mutate_run(&request("run-9"))
            .await
            .unwrap();
        assert_eq!(report.failure, None, "{report:#?}");
        assert_eq!(
            (report.sites, report.planned, report.mutants.len()),
            (5, 3, 3)
        );
    }

    #[tokio::test]
    async fn a_baseline_that_fails_is_a_failure_and_the_provider_is_never_asked() {
        let project =
            Project::new(&TABLE.replace("grep -q HARNESS check.txt", "grep -q NOPE check.txt"));
        let report = project
            .runner()
            .mutate_run(&request("run-2"))
            .await
            .unwrap();
        let failure = report.failure.expect("a failure");
        assert_eq!(failure.kind(), FailureKind::Baseline);
        assert!(failure.message().contains("baseline-test"), "{failure}");
        assert!(report.mutants.is_empty());
        let steps: Vec<&str> = report.evidence.iter().map(|e| e.step.as_str()).collect();
        assert_eq!(steps, ["baseline-build", "baseline-test"]);
        assert_eq!(report.evidence[1].exit_code, Some(1));
        assert!(!project.0.join("provider-called").exists());
        assert!(!project.0.join(WORK_DIR).join("run-2").exists());
    }

    #[tokio::test]
    async fn a_mutant_that_does_not_end_in_time_is_a_timeout_and_the_run_goes_on() {
        let slow = TABLE.replace("timeout = 5", "timeout = 1").replace(
            "grep -q 'a <= ' code.txt",
            "if grep -q 'a <= 6' code.txt; then sleep 30; fi; grep -q 'a <= ' code.txt",
        );
        let project = Project::new(&slow);
        let started = Instant::now();
        let report = project
            .runner()
            .mutate_run(&request("run-3"))
            .await
            .unwrap();
        assert_eq!(report.failure, None, "{report:#?}");
        assert_eq!(
            outcomes(&report),
            [
                ("f/rel/1/0000", "killed"),
                ("f/arith/1/0000", "stillborn"),
                ("f/int/1/0000", "timeout"),
            ]
        );
        assert!(started.elapsed().as_secs() < 20);
    }

    #[tokio::test]
    async fn a_run_that_uses_up_its_budget_fails_with_the_mutants_it_got_through() {
        let slow = TABLE
            .replace("timeout = 5", "timeout = 30\nbudget = 2")
            .replace(
                "grep -q 'a <= ' code.txt",
                "if grep -q 'BROKEN' code.txt; then :; elif grep -q 'a < 5' code.txt; then sleep 30; fi; grep -q 'a <= ' code.txt",
            );
        let project = Project::new(&slow);
        let report = project
            .runner()
            .mutate_run(&request("run-4"))
            .await
            .unwrap();
        let failure = report.failure.expect("a failure");
        assert_eq!(failure.kind(), FailureKind::Budget);
        assert!(report.mutants.is_empty());
        // the provider had answered: what the run owed is on record beside what it got through
        assert_eq!((report.sites, report.planned), (3, 3));
        assert!(!project.0.join(WORK_DIR).join("run-4").exists());
    }

    #[tokio::test]
    async fn a_provider_that_disagrees_with_the_source_is_a_protocol_failure() {
        let project = Project::new(TABLE);
        // the baseline passes (`a <= ` is there), and the provider's second edit is of a `+` that is a `-` here
        let mut r = request("run-6");
        r.code = "a <= 5 and b - 1\n".to_string();
        let report = project.runner().mutate_run(&r).await.unwrap();
        let failure = report.failure.expect("a failure");
        assert_eq!(failure.kind(), FailureKind::Protocol, "{failure}");
        assert!(failure.message().contains("is not `+`"), "{failure}");
        assert!(report.mutants.is_empty());
    }

    #[tokio::test]
    async fn a_configuration_that_cannot_measure_this_is_an_error_and_runs_nothing() {
        let project = Project::new(TABLE);
        let runner = project.runner();
        let mut unknown = request("run-7");
        unknown.interface = asset("other");
        let error = runner.mutate_run(&unknown).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no entry for the interface `other`")
        );
        let mut no_harness = request("run-8");
        no_harness.harnesses[0].name = asset("elsewhere");
        let error = runner.mutate_run(&no_harness).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no entry for the harness `elsewhere`")
        );
        let error = runner.mutate_run(&request("../escape")).await.unwrap_err();
        assert!(error.to_string().contains("not a name"));
        assert!(!project.0.join(WORK_DIR).exists());
    }

    /// The digest of the settings of a mutation run of `api` that runs the tests `checks`.
    async fn digest(project: &Project) -> SettingsDigest {
        project
            .runner()
            .settings(&context("ctx"), &asset("api"), &[asset("checks")])
            .await
            .unwrap()
    }

    /// [`TABLE`] with the top-level keys `keys` before its first table.
    fn with(keys: &str) -> String {
        format!("{keys}\n{TABLE}")
    }

    /// [`TABLE`] with `text` after its last layout entry.
    fn plus(text: &str) -> String {
        format!("{TABLE}{text}")
    }

    #[tokio::test]
    async fn a_setting_a_mutation_run_does_not_read_leaves_its_digest_as_it_was() {
        let project = Project::new(TABLE);
        let before = digest(&project).await;
        assert_eq!(before, digest(&project).await);
        // the layout entries in the other order
        let mut parts: Vec<&str> = TABLE.split("[[layout]]").collect();
        parts.swap(1, 2);
        let swapped = parts.join("[[layout]]");
        assert_ne!(swapped, TABLE);
        for (what, table) in [
            (
                "a provider of items",
                TABLE.replace("[mutants]", "[items]\ntext = [\"./items.sh\"]\n[mutants]"),
            ),
            (
                "a sim of the context",
                plus("[measure.sim.ctx]\ninterpreter = [\"sh\"]\nprogram = [\"x\"]\n"),
            ),
            (
                "a provider of another language",
                TABLE.replace(
                    "text = [\"./provider.sh\"]",
                    "text = [\"./provider.sh\"]\ngo = [\"./other.sh\"]",
                ),
            ),
            (
                "another interface laid out",
                plus(
                    "[[layout]]\ncontext = \"ctx\"\ninterface = \"elsewhere\"\nlang = \"text\"\npath = \"else.txt\"\n",
                ),
            ),
            (
                "a harness the run does not run",
                plus("[[layout]]\ncontext = \"ctx\"\nharness = \"unrun\"\npath = \"unrun.txt\"\n"),
            ),
            (
                "the layout of another context",
                plus("[[layout]]\ncontext = \"other\"\ninterface = \"api\"\npath = \"o.txt\"\n"),
            ),
            ("the layout in another order", swapped),
        ] {
            project.table(&table);
            assert_eq!(digest(&project).await, before, "{what}");
        }
    }

    #[tokio::test]
    async fn each_setting_a_mutation_run_reads_moves_its_digest() {
        let project = Project::new(TABLE);
        let before = digest(&project).await;
        let mut seen = vec![before];
        for (what, table) in [
            ("copy_exclude", with("copy_exclude = [\".git\"]")),
            ("env", TABLE.replace("env = {}", "env = { A = \"1\" }")),
            (
                "build",
                TABLE.replace("! grep -q BROKEN", "! grep -q BROKEN2"),
            ),
            ("baseline_timeout", with("baseline_timeout = 7")),
            ("test", TABLE.replace("grep -q HARNESS", "grep -q HARNESS2")),
            ("timeout", TABLE.replace("timeout = 5", "timeout = 6")),
            ("budget", with("budget = 800")),
            ("max_mutants", with("max_mutants = 3")),
            ("seed", with("seed = 9")),
            ("operators", with("operators = [\"rel\"]")),
            (
                "the provider's command",
                TABLE.replace(
                    "text = [\"./provider.sh\"]",
                    "text = [\"./provider.sh\", \"--x\"]",
                ),
            ),
            (
                "the path of the interface",
                TABLE.replace("path = \"code.txt\"", "path = \"code2.txt\""),
            ),
            (
                "the path of the harness",
                TABLE.replace("path = \"check.txt\"", "path = \"check2.txt\""),
            ),
            (
                "the language of the interface",
                TABLE
                    .replace(
                        "text = [\"./provider.sh\"]",
                        "text = [\"./provider.sh\"]\ntext2 = [\"./provider.sh\"]",
                    )
                    .replace("lang = \"text\"", "lang = \"text2\""),
            ),
        ] {
            project.table(&table);
            let now = digest(&project).await;
            assert!(!seen.contains(&now), "{what} moves the digest");
            seen.push(now);
        }
    }

    #[tokio::test]
    async fn the_tests_a_run_runs_are_in_its_digest_in_whatever_order_they_are_given() {
        let table =
            plus("[[layout]]\ncontext = \"ctx\"\nharness = \"more\"\npath = \"more.txt\"\n");
        let project = Project::new(&table);
        let runner = project.runner();
        async fn of(runner: &ProcessRunner, names: &[&str]) -> SettingsDigest {
            let names: Vec<AssetName> = names.iter().map(|n| asset(n)).collect();
            runner
                .settings(&context("ctx"), &asset("api"), &names)
                .await
                .unwrap()
        }
        let both = of(&runner, &["checks", "more"]).await;
        assert_eq!(both, of(&runner, &["more", "checks"]).await);
        assert_ne!(both, of(&runner, &["checks"]).await);
        assert_ne!(of(&runner, &["checks"]).await, of(&runner, &["more"]).await);
        assert_ne!(of(&runner, &["checks"]).await, of(&runner, &[]).await);
    }

    #[tokio::test]
    async fn a_table_with_its_defaults_written_out_and_one_without_have_one_digest() {
        // `TABLE` says `env = {}`, which is no variable at all and not the default one
        let bare = TABLE.replace("env = {}\n", "");
        let project = Project::new(&bare);
        let before = digest(&project).await;
        let written = format!(
            "copy_exclude = [\"target\", \".chidev\", \".git\"]\nenv = {{ CARGO_TARGET_DIR = \"{{run}}/target\" }}\nbaseline_timeout = 600\nbudget = 900\nmax_mutants = 40\nseed = 0\n{bare}"
        );
        project.table(&written);
        assert_eq!(digest(&project).await, before);
        // and the default is not the table that says there is no variable
        project.table(TABLE);
        assert_ne!(digest(&project).await, before);
    }

    impl Project {
        /// Writes the executable `name` of the project.
        fn script(&self, name: &str, text: &str) {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(self.0.join(name), text).unwrap();
            std::fs::set_permissions(self.0.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }

    /// [`TABLE`] with the build and the test commands scripts of the project.
    fn scripted() -> String {
        TABLE
            .replace(
                "build = [\"sh\", \"-c\", \"! grep -q BROKEN code.txt\"]",
                "build = [\"./build.sh\"]",
            )
            .replace(
                "test = [\"sh\", \"-c\", \"grep -q 'a <= ' code.txt && grep -q HARNESS check.txt\"]",
                "test = [\"./test.sh\"]",
            )
    }

    #[tokio::test]
    async fn the_programs_a_run_starts_are_in_its_digest_by_their_files_and_not_by_where_the_project_is()
     {
        assert_ne!(scripted(), TABLE);
        let project = Project::new(&scripted());
        project.script("build.sh", "#!/bin/sh\ntrue\n");
        project.script("test.sh", "#!/bin/sh\ntrue\n");
        let before = digest(&project).await;
        // the same project somewhere else
        let elsewhere = Project::new(&scripted());
        elsewhere.script("build.sh", "#!/bin/sh\ntrue\n");
        elsewhere.script("test.sh", "#!/bin/sh\ntrue\n");
        assert_ne!(project.0, elsewhere.0);
        assert_eq!(digest(&elsewhere).await, before);
        // each program, changed in its file under the same table
        let mut seen = vec![before];
        for (what, name, text) in [
            (
                "the build program",
                "build.sh",
                "#!/bin/sh\ntrue # changed\n",
            ),
            ("the test program", "test.sh", "#!/bin/sh\ntrue # changed\n"),
            ("the provider", "provider.sh", "#!/bin/sh\n# changed\n"),
        ] {
            project.script(name, text);
            let now = digest(&project).await;
            assert!(!seen.contains(&now), "{what} moves the digest");
            seen.push(now);
        }
    }

    #[tokio::test]
    async fn a_report_carries_the_digest_of_the_settings_the_run_resolved_and_ran_under() {
        let project = Project::new(TABLE);
        let report = project
            .runner()
            .mutate_run(&request("run-digest"))
            .await
            .unwrap();
        assert_eq!(report.failure, None, "{report:#?}");
        assert_eq!(report.settings, digest(&project).await);
        // the run resolved the table again: another table gives another digest in the report
        project.table(&TABLE.replace("timeout = 5", "timeout = 6"));
        let other = project
            .runner()
            .mutate_run(&request("run-digest-2"))
            .await
            .unwrap();
        assert_ne!(other.settings, report.settings);
        assert_eq!(other.settings, digest(&project).await);
    }

    #[tokio::test]
    async fn a_program_that_is_another_file_in_the_copy_than_at_the_root_fails_the_run_as_settings_moved()
     {
        // the Harness is written over `build.sh` in the copy, where the build command finds the Harness's text
        // under the name of the program the settings were digested with
        let table = scripted().replace("path = \"check.txt\"", "path = \"build.sh\"");
        let project = Project::new(&table);
        project.script("build.sh", "#!/bin/sh\ntrue\n");
        project.script("test.sh", "#!/bin/sh\ntrue\n");
        let report = project
            .runner()
            .mutate_run(&request("run-moved"))
            .await
            .unwrap();
        let failure = report.failure.expect("a failure");
        assert_eq!(failure.kind(), FailureKind::SettingsMoved, "{failure}");
        assert!(failure.message().contains("`./build.sh`"), "{failure}");
        // nothing was run: no command, no provider
        assert!(report.evidence.is_empty() && report.mutants.is_empty());
        assert!(!project.0.join("provider-called").exists());
        assert!(!project.0.join(WORK_DIR).join("run-moved").exists());
        // the same table with the Harness elsewhere is a run
        let elsewhere = Project::new(&scripted());
        elsewhere.script("build.sh", "#!/bin/sh\ntrue\n");
        elsewhere.script("test.sh", "#!/bin/sh\ntrue\n");
        let report = elsewhere
            .runner()
            .mutate_run(&request("run-fine"))
            .await
            .unwrap();
        assert_eq!(report.failure, None, "{report:#?}");
    }

    #[tokio::test]
    async fn a_provider_that_is_another_file_when_it_is_started_fails_the_run_as_settings_moved() {
        // the baseline build changes the provider at the project's root after the settings were digested
        let project = Project::new(TABLE);
        let table = TABLE
            .replace(
                "env = {}",
                &format!(
                    "env = {{ PROVIDER = \"{}\" }}",
                    project.0.join("provider.sh").display()
                ),
            )
            .replace(
                "build = [\"sh\", \"-c\", \"! grep -q BROKEN code.txt\"]",
                "build = [\"sh\", \"-c\", \"echo '# moved' >> \\\"$PROVIDER\\\"\"]",
            );
        assert_ne!(table, TABLE);
        project.table(&table);
        let report = project
            .runner()
            .mutate_run(&request("run-provider-moved"))
            .await
            .unwrap();
        let failure = report.failure.expect("a failure");
        assert_eq!(failure.kind(), FailureKind::SettingsMoved, "{failure}");
        assert!(failure.message().contains("`./provider.sh`"), "{failure}");
        assert!(report.mutants.is_empty());
    }

    #[tokio::test]
    async fn a_program_the_build_rewrites_in_the_copy_is_the_runs_and_not_a_moved_setting() {
        // the build writes `test.sh` in the copy, after the copy is made and its programs were found as digested
        let project = Project::new(&scripted());
        project.script(
            "build.sh",
            "#!/bin/sh\nprintf '#!/bin/sh\\ntrue # rebuilt\\n' > test.sh\n",
        );
        project.script("test.sh", "#!/bin/sh\ntrue\n");
        let report = project
            .runner()
            .mutate_run(&request("run-rebuilt"))
            .await
            .unwrap();
        assert_eq!(report.failure, None, "{report:#?}");
        assert_eq!(report.mutants.len(), 3);
    }

    #[test]
    fn a_job_holds_the_root_the_shared_settings_and_its_places_and_not_the_runner_or_the_table() {
        // Every field is named, so a field added (the runner, the table) does not compile until it is named here.
        fn fields(job: Job<'_>) {
            let Job {
                root,
                copy,
                dirs,
                deadline,
            } = job;
            let _: &Path = root;
            let _: &CopySettings = copy;
            let _: RunDirs = dirs;
            let _: tokio::time::Instant = deadline;
        }
        let _ = fields;
    }

    /// The error that the settings of a mutation run of `api` that runs `checks` give for `table`.
    async fn refused(table: &str) -> RunnerError {
        Project::new(table)
            .runner()
            .settings(&context("ctx"), &asset("api"), &[asset("checks")])
            .await
            .unwrap_err()
    }

    /// The program no table names, so that it is not found.
    const MISSING: &str = "chidev-no-such-program";

    #[tokio::test]
    async fn an_interface_with_no_layout_entry_has_no_layout() {
        let project = Project::new(TABLE);
        let error = project
            .runner()
            .settings(&context("ctx"), &asset("other"), &[])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::NoLayout {
                place: LayoutPlace::Interface,
                context: context("ctx"),
                name: asset("other"),
            }
        );
        assert_eq!(
            error.to_string(),
            "[[measure.layout]] has no entry for the interface `other` of `ctx`"
        );
    }

    #[tokio::test]
    async fn a_harness_with_no_layout_entry_has_no_layout() {
        let project = Project::new(TABLE);
        let error = project
            .runner()
            .settings(&context("ctx"), &asset("api"), &[asset("elsewhere")])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::NoLayout {
                place: LayoutPlace::Harness,
                context: context("ctx"),
                name: asset("elsewhere"),
            }
        );
        assert_eq!(
            error.to_string(),
            "[[measure.layout]] has no entry for the harness `elsewhere` of `ctx`"
        );
    }

    #[tokio::test]
    async fn an_interface_laid_out_without_a_language_has_no_lang() {
        let project = Project::new(ITEMS_TABLE);
        let error = project
            .runner()
            .settings(&context("ctx"), &asset("bare"), &[])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::NoLang {
                interface: asset("bare")
            }
        );
        assert_eq!(
            error.to_string(),
            "the layout of interface `bare` has no `lang`, which mutating its code needs"
        );
    }

    #[tokio::test]
    async fn a_language_with_no_command_in_mutants_has_no_provider() {
        // `ITEMS_TABLE` has a command for `text` in `items` alone
        let project = Project::new(ITEMS_TABLE);
        let error = project
            .runner()
            .settings(&context("ctx"), &asset("api"), &[])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::NoProvider {
                lang: "text".to_string()
            }
        );
        assert_eq!(error.to_string(), "`mutants` has no command for `text`");
    }

    #[tokio::test]
    async fn a_table_with_no_test_has_no_test_command_for_a_mutation_run() {
        let table = TABLE.replace(
            "test = [\"sh\", \"-c\", \"grep -q 'a <= ' code.txt && grep -q HARNESS check.txt\"]\n",
            "",
        );
        assert_ne!(table, TABLE);
        let error = refused(&table).await;
        assert_eq!(error, RunnerError::NoTestCommand);
        assert_eq!(
            error.to_string(),
            "[measure] has no `test` command to mutate with"
        );
    }

    #[tokio::test]
    async fn a_context_with_no_sim_table_has_no_sim_config() {
        let project = Project::new(TABLE);
        let error = project
            .runner()
            .sim_settings(&context("ctx"), &asset("checks"), &[])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RunnerError::NoSimConfig {
                context: context("ctx")
            }
        );
        assert_eq!(
            error.to_string(),
            "[measure.sim.ctx] is not configured; it gives the interpreter and the program"
        );
    }

    #[tokio::test]
    async fn a_program_that_is_not_found_is_not_located_and_says_whose_it_is() {
        let build = r#"build = ["sh", "-c", "! grep -q BROKEN code.txt"]"#;
        let test =
            r#"test = ["sh", "-c", "grep -q 'a <= ' code.txt && grep -q HARNESS check.txt"]"#;
        for (what, table) in [
            (
                CommandRole::Build,
                TABLE.replace(build, &format!("build = [\"{MISSING}\"]")),
            ),
            (
                CommandRole::Test,
                TABLE.replace(test, &format!("test = [\"{MISSING}\"]")),
            ),
            (
                CommandRole::Provider {
                    lang: "text".to_string(),
                },
                TABLE.replace("./provider.sh", MISSING),
            ),
        ] {
            assert_ne!(table, TABLE);
            let error = refused(&table).await;
            let said = format!("executable `{MISSING}` not found in PATH");
            assert_eq!(
                error,
                RunnerError::Locate {
                    what: what.clone(),
                    reason: said.clone()
                }
            );
            assert_eq!(error.to_string(), format!("{what}: {said}"));
        }
        // the provider's message names the language, as it always has
        let error = refused(&TABLE.replace("./provider.sh", MISSING)).await;
        assert_eq!(
            error.to_string(),
            format!("the provider for `text`: executable `{MISSING}` not found in PATH")
        );
        // the interpreter of a sim
        let sim = plus(&format!(
            "[measure.sim.ctx]\ninterpreter = [\"{MISSING}\"]\nprogram = [\"x\"]\n"
        ));
        let project = Project::new(&sim);
        let error = project
            .runner()
            .sim_settings(&context("ctx"), &asset("checks"), &[])
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("the interpreter: executable `{MISSING}` not found in PATH")
        );
        assert!(matches!(
            error,
            RunnerError::Locate {
                what: CommandRole::Interpreter,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_program_that_is_found_and_is_not_a_file_cannot_be_fingerprinted() {
        let table = TABLE.replace(
            r#"build = ["sh", "-c", "! grep -q BROKEN code.txt"]"#,
            "build = [\"./adir\"]",
        );
        let project = Project::new(&table);
        std::fs::create_dir(project.0.join("adir")).unwrap();
        let error = project
            .runner()
            .settings(&context("ctx"), &asset("api"), &[asset("checks")])
            .await
            .unwrap_err();
        let said = "the configured executable is not a regular file";
        assert_eq!(
            error,
            RunnerError::Fingerprint {
                what: CommandRole::Build,
                reason: said.to_string()
            }
        );
        assert_eq!(error.to_string(), format!("the build command: {said}"));
    }

    /// [`scripted`] with a second Interface `web` of a language whose provider is the same binary as `api`'s with
    /// another argument, and a sim and examples of the Context whose program is the host's `sh`.
    fn shared() -> String {
        let table = scripted().replace(
            "text = [\"./provider.sh\"]",
            "text = [\"./provider.sh\"]\ntext2 = [\"./provider.sh\", \"--x\"]",
        );
        format!(
            "{table}[[layout]]\ncontext = \"ctx\"\ninterface = \"web\"\nlang = \"text2\"\npath = \"web.txt\"\n\
             [measure.sim.ctx]\ninterpreter = [\"sh\"]\nprogram = [\"sh\", \"-s\"]\n\
             [measure.examples.ctx]\nprogram = [\"sh\", \"-s\"]\n"
        )
    }

    fn shared_project() -> Project {
        let project = Project::new(&shared());
        project.script("build.sh", "#!/bin/sh\ntrue\n");
        project.script("test.sh", "#!/bin/sh\ntrue\n");
        project
    }

    /// One target of each kind, and a second of the mutation kind.
    fn targets() -> Vec<SettingsTarget> {
        vec![
            SettingsTarget::Mutation {
                context: context("ctx"),
                interface: asset("api"),
                harnesses: vec![asset("checks")],
            },
            SettingsTarget::Mutation {
                context: context("ctx"),
                interface: asset("web"),
                harnesses: vec![asset("checks")],
            },
            SettingsTarget::Simulation {
                context: context("ctx"),
                harness: asset("checks"),
                builds: vec![asset("api"), asset("web")],
            },
            SettingsTarget::Examples {
                context: context("ctx"),
                harness: asset("ex"),
                builds: vec![asset("api")],
            },
        ]
    }

    /// What the single ask of the kind of `target` says of it.
    async fn alone(
        runner: &ProcessRunner,
        target: &SettingsTarget,
    ) -> Result<SettingsDigest, RunnerError> {
        match target {
            SettingsTarget::Mutation {
                context,
                interface,
                harnesses,
            } => runner.settings(context, interface, harnesses).await,
            SettingsTarget::Simulation {
                context,
                harness,
                builds,
            } => runner.sim_settings(context, harness, builds).await,
            SettingsTarget::Examples {
                context,
                harness,
                builds,
            } => runner.examples_settings(context, harness, builds).await,
        }
    }

    #[tokio::test]
    async fn subjects_that_share_programs_have_each_file_hashed_once_in_one_ask() {
        let project = shared_project();
        let runner = project.runner();
        let targets = targets();
        // two Interfaces that share the build, the test and the provider's file: three files, not six
        let mut together = Files::default();
        for target in &targets[..2] {
            runner
                .digest_of_target(&mut together, target)
                .await
                .unwrap();
        }
        assert_eq!(together.read(), 3);
        let mut apart = 0;
        for target in &targets[..2] {
            let mut files = Files::default();
            runner.digest_of_target(&mut files, target).await.unwrap();
            apart += files.read();
        }
        assert_eq!(apart, 6);
        // the sim and the examples add the host's `sh` and nothing of the build they share
        for target in &targets[2..] {
            runner
                .digest_of_target(&mut together, target)
                .await
                .unwrap();
        }
        assert!(
            (4..=5).contains(&together.read()),
            "{} files",
            together.read()
        );
        // and what is said is what the single asks say, for every target
        let said = runner.settings_of(&targets).await;
        assert_eq!(said.len(), targets.len());
        for (target, digest) in &said {
            assert!(digest.is_ok(), "{target:?}: {digest:?}");
            assert_eq!(digest, &alone(&runner, target).await, "{target:?}");
        }
    }

    #[tokio::test]
    async fn the_answers_are_paired_with_their_targets_whatever_the_order_they_are_asked_in() {
        let project = shared_project();
        let runner = project.runner();
        let gone = SettingsTarget::Mutation {
            context: context("ctx"),
            interface: asset("elsewhere"),
            harnesses: Vec::new(),
        };
        let mut forward = targets();
        forward.insert(2, gone.clone());
        let backward: Vec<SettingsTarget> = forward.iter().rev().cloned().collect();
        let (first, second) = (
            runner.settings_of(&forward).await,
            runner.settings_of(&backward).await,
        );
        assert_eq!(
            first.iter().map(|(t, _)| t).collect::<Vec<_>>(),
            forward.iter().collect::<Vec<_>>()
        );
        assert_eq!(
            second.iter().map(|(t, _)| t).collect::<Vec<_>>(),
            backward.iter().collect::<Vec<_>>()
        );
        for (target, digest) in &first {
            let (_, other) = second.iter().find(|(t, _)| t == target).unwrap();
            assert_eq!(digest, other, "{target:?}");
        }
        // the one that cannot be read says so beside its own target, and does not take the others with it
        let unread: Vec<_> = first.iter().filter(|(_, d)| d.is_err()).collect();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].0, gone);
        assert!(matches!(
            unread[0].1,
            Err(RunnerError::NoLayout {
                place: LayoutPlace::Interface,
                ..
            })
        ));
        // each of the others has a digest of its own
        let digests: Vec<_> = first.iter().filter_map(|(_, d)| d.as_ref().ok()).collect();
        assert_eq!(digests.len(), 4);
        for (i, one) in digests.iter().enumerate() {
            for other in &digests[i + 1..] {
                assert_ne!(one, other);
            }
        }
    }

    #[tokio::test]
    async fn one_binary_started_by_two_commands_with_other_arguments_keeps_a_digest_for_each() {
        let project = shared_project();
        let runner = project.runner();
        let mut files = Files::default();
        let [api, web] = ["api", "web"].map(asset);
        let harnesses = [asset("checks")];
        let api = MutationSettings::resolve(
            &mut files,
            &runner.config,
            &runner.root,
            &context("ctx"),
            &api,
            &harnesses,
        )
        .await
        .unwrap();
        let web = MutationSettings::resolve(
            &mut files,
            &runner.config,
            &runner.root,
            &context("ctx"),
            &web,
            &harnesses,
        )
        .await
        .unwrap();
        // the file is the same and read once, with the build and the test: three files
        assert_eq!(api.provider.executable(), web.provider.executable());
        assert_eq!(files.read(), 3);
        // the commands are not: each keeps its own arguments
        assert!(api.provider.args().is_empty());
        assert_eq!(web.provider.args(), ["--x"]);
        assert_ne!(api.provider, web.provider);
        // and so, in one ask, the settings of the two are two digests
        let said = runner.settings_of(&targets()[..2]).await;
        assert_ne!(said[0].1.as_ref().unwrap(), said[1].1.as_ref().unwrap());
        assert_eq!(said[1].1, alone(&runner, &said[1].0).await);
        // a table that differs from this one by the arguments alone gives another digest of the same Interface
        async fn of_web(project: &Project) -> SettingsDigest {
            let ask = targets()[1..2].to_vec();
            let mut said = project.runner().settings_of(&ask).await;
            said.remove(0).1.unwrap()
        }
        let before = of_web(&project).await;
        project.table(&shared().replace("\"--x\"", "\"--y\""));
        assert_ne!(of_web(&project).await, before);
    }

    #[tokio::test]
    async fn nothing_is_kept_between_two_asks() {
        let project = shared_project();
        let runner = project.runner();
        let targets = &targets()[..1];
        let before = runner.settings_of(targets).await;
        // the file of the build changes between the asks, and the runner that was asked before sees it
        project.script("build.sh", "#!/bin/sh\ntrue # changed\n");
        let after = runner.settings_of(targets).await;
        assert_ne!(before[0].1.as_ref().unwrap(), after[0].1.as_ref().unwrap());
    }

    #[tokio::test]
    async fn a_name_that_leads_out_of_the_runs_directory_is_a_bad_run_name() {
        let project = Project::new(TABLE);
        let error = project
            .runner()
            .mutate_run(&request("../escape"))
            .await
            .unwrap_err();
        assert_eq!(error, RunnerError::BadRunName("../escape".to_string()));
        assert_eq!(
            error.to_string(),
            "`../escape` is not a name for a run's directory"
        );
        assert!(!project.0.join(WORK_DIR).exists());
    }

    #[test]
    fn a_limit_the_clock_cannot_count_says_so() {
        // `budget` is capped at an hour by the table, so no table gives this; the error is the clock's own word
        let failure = process::deadline_in(std::time::Duration::MAX).unwrap_err();
        let error = RunnerError::Deadline(failure.message);
        assert_eq!(error.to_string(), "the timeout exceeds the clock's range");
    }

    /// A table whose `api` is a text laid out with a language that only `items` has a command for, and whose
    /// `bare` has no language.
    const ITEMS_TABLE: &str = r#"
build = ["true"]
timeout = 5
env = {}
[items]
text = ["./items.sh"]
[[layout]]
context = "ctx"
interface = "api"
lang = "text"
path = "code.txt"
[[layout]]
context = "ctx"
interface = "bare"
path = "bare.txt"
"#;

    /// A provider that keeps what it was asked and answers two Items on the one line.
    const ITEMS: &str = r#"#!/bin/sh
cat > request.json
printf '%s' '{"protocol":1,"items":[{"key":"api","lines":[1,1]},{"key":"api::f","lines":[1,1]}]}'
"#;

    impl Project {
        /// Writes the provider of items `script`, executable.
        fn items_script(&self, script: &str) {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(self.0.join("items.sh"), script).unwrap();
            std::fs::set_permissions(
                self.0.join("items.sh"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn the_items_of_a_language_are_asked_of_its_provider_in_the_root_and_taken_as_given() {
        let project = Project::new(ITEMS_TABLE);
        project.items_script(ITEMS);
        let runner = project.runner();
        // the language is the layout's, and an interface with none (or none laid out) has no language
        let ctx = context("ctx");
        assert_eq!(runner.language(&ctx, &asset("api")), Ok("text".to_string()));
        assert_eq!(
            runner.language(&ctx, &asset("bare")),
            Err(ItemsError::NoLanguage)
        );
        assert_eq!(
            runner.language(&ctx, &asset("nowhere")),
            Err(ItemsError::NoLanguage)
        );
        let found = runner.items_of("text", "fn api() { f() }\n").await.unwrap();
        assert_eq!(
            found
                .iter()
                .map(|i| (i.key.as_str(), i.lines))
                .collect::<Vec<_>>(),
            [("api", (1, 1)), ("api::f", (1, 1))]
        );
        // the request is the protocol, and the provider ran where the table is
        let asked: serde_json::Value =
            serde_json::from_slice(&std::fs::read(project.0.join("request.json")).unwrap())
                .unwrap();
        assert_eq!(
            asked,
            serde_json::json!({"protocol": 1, "source": "fn api() { f() }\n"})
        );
        // a language `items` has no command for has no provider
        assert_eq!(
            runner.items_of("go", "x\n").await,
            Err(ItemsError::NoProvider)
        );
    }

    #[tokio::test]
    async fn a_provider_that_cannot_be_run_to_its_end_has_failed_and_one_that_disagrees_has_not_kept_the_protocol()
     {
        let project = Project::new(ITEMS_TABLE);
        let failed = |e: Result<Vec<FoundItem>, ItemsError>| match e {
            Err(ItemsError::Failed(m)) => m,
            other => panic!("not a failure to run: {other:?}"),
        };
        // not found
        let said = failed(project.runner().items_of("text", "x\n").await);
        assert!(said.contains("spawn"), "{said}");
        // exits with a failure, and says why on standard error
        project.items_script("#!/bin/sh\ncat > /dev/null\necho 'cannot parse' >&2\nexit 3\n");
        let said = failed(project.runner().items_of("text", "x\n").await);
        assert!(said.contains("cannot parse"), "{said}");
        // does not answer in time
        project.table(&ITEMS_TABLE.replace("timeout = 5", "timeout = 1"));
        project.items_script("#!/bin/sh\nsleep 30\n");
        let started = Instant::now();
        let said = failed(project.runner().items_of("text", "x\n").await);
        assert!(said.contains("in time"), "{said}");
        assert!(started.elapsed().as_secs() < 20);
        // answers what is not a reply, or a reply that disagrees with the code
        project.table(ITEMS_TABLE);
        for (answer, expected) in [
            ("not json", "not a chidev-items reply"),
            (
                r#"{"protocol":1,"items":[{"key":"a","lines":[1,2]}]}"#,
                "past the last",
            ),
        ] {
            project.items_script(&format!(
                "#!/bin/sh\ncat > /dev/null\nprintf '%s' '{answer}'\n"
            ));
            match project.runner().items_of("text", "x\n").await {
                Err(ItemsError::Protocol(m)) => assert!(m.contains(expected), "{m}"),
                other => panic!("not a protocol failure: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_language_only_items_has_a_command_for_is_stopped_by_name_when_mutating() {
        let project = Project::new(ITEMS_TABLE);
        project.items_script(ITEMS);
        let mut r = request("run-items");
        r.interface = asset("api");
        let error = project.runner().mutate_run(&r).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`mutants` has no command for `text`"),
            "{error}"
        );
        let error = project
            .runner()
            .settings(&context("ctx"), &asset("api"), &[])
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`mutants` has no command for `text`"),
            "{error}"
        );
        assert!(!project.0.join(WORK_DIR).exists());
    }

    #[test]
    fn a_project_without_the_table_has_no_runner_and_a_wrong_table_is_an_error() {
        let project = Project::new(TABLE);
        std::fs::write(project.0.join(FILE), "[checks]\n").unwrap();
        assert!(load(&project.0).unwrap().is_none());
        std::fs::write(project.0.join(FILE), "").unwrap();
        assert!(load(&project.0).unwrap().is_none());
        std::fs::write(project.0.join(FILE), "[measure]\nbuild = []\ntest = []\n").unwrap();
        let error = load(&project.0).unwrap_err();
        assert!(
            error.to_string().contains("measure: `build` is a command"),
            "{error}"
        );
    }

    #[test]
    fn the_copy_leaves_out_what_it_is_told_and_its_own_directory_and_follows_no_link() {
        let project = Project::new(TABLE);
        std::fs::create_dir_all(project.0.join("src/deep")).unwrap();
        std::fs::write(project.0.join("src/deep/a.txt"), "a").unwrap();
        std::fs::create_dir_all(project.0.join(WORK_DIR).join("old")).unwrap();
        std::os::unix::fs::symlink("/etc", project.0.join("link")).unwrap();
        let to = project.0.join("copied");
        copy_tree(
            &project.0,
            &to,
            &[PathBuf::from("target"), PathBuf::from("copied")],
        )
        .unwrap();
        assert!(to.join("src/deep/a.txt").exists());
        assert!(to.join("code.txt").exists());
        assert!(!to.join("target").exists());
        // the directory runs work in is never copied, whatever the file excludes
        assert!(to.join(".chidev").exists());
        assert!(!to.join(".chidev/measure").exists());
        assert!(!to.join("link").exists());
    }
}
