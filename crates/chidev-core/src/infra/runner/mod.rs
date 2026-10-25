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
//! A run that cannot measure (the baseline fails, a command or the provider fails, the budget is used up) still
//! reports what it observed: the failure, the commands' evidence and the mutants it got through. Only a
//! configuration that cannot measure this Interface at all is an error, and then nothing was run.
//!
//! A simulation (`sim.rs`, which holds the protocol `chidev-sim 2`) is run in the same kind of copy: the pinned code
//! of the Interfaces the sim Harness builds and the script are written over it, the build command runs, and the
//! script is started once per case under the protocol `chidev-sim 2`. The sampled cases are then run a second time
//! in a second, fresh copy.
//!
//! The same configuration answers a second question for the use cases, as the [`ItemSource`] port: the language of an
//! Interface (its layout's `lang`) and the Items a provider of that language finds in its code (`items.rs`, which
//! holds the protocol `chidev-items 1`). That provider is started in the project's root, not in a copy, under the
//! `timeout` of the table.

mod config;
mod items;
mod provider;
mod sim;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use thiserror::Error;

use self::config::{Config, RunDirs, WORK_DIR};
use super::process::{self, Capture, Over, Pipe};
use super::project::{Project, Section};
use crate::app::ports::items::{FoundItem, ItemSource, ItemsError};
use crate::app::ports::measure::{
    BoxFuture, MutateReport, MutateRequest, Runner, RunnerError, SimulateReport, SimulateRequest,
};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::measure::{
    FailureKind, MeasureFailure, MutantOutcome, MutantResult, StepEvidence,
};

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
    fn settings<'a>(
        &'a self,
        context: &'a str,
        interface: &'a str,
    ) -> BoxFuture<'a, Result<String, RunnerError>> {
        Box::pin(self.settings_of(context, interface))
    }

    fn mutate<'a>(
        &'a self,
        request: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(self.mutate_run(request))
    }

    fn sim_settings<'a>(
        &'a self,
        context: &'a str,
        harness: &'a str,
    ) -> BoxFuture<'a, Result<String, RunnerError>> {
        Box::pin(self.sim_settings_of(context, harness))
    }

    fn simulate<'a>(
        &'a self,
        request: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        Box::pin(self.simulate_run(request))
    }
}

impl ItemSource for ProcessRunner {
    fn language(&self, context: &str, interface: &str) -> Result<String, ItemsError> {
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

fn config_error(message: String) -> RunnerError {
    RunnerError::Config(message)
}

fn needs_lang(interface: &str) -> RunnerError {
    config_error(format!(
        "the layout of interface `{interface}` has no `lang`, which mutating its code needs"
    ))
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
    async fn settings_of(&self, context: &str, interface: &str) -> Result<String, RunnerError> {
        let (_, lang) = self.config.interface(context, interface).ok_or_else(|| {
            config_error(format!(
                "[[measure.layout]] has no entry for the interface `{interface}` of `{context}`"
            ))
        })?;
        let lang = lang.ok_or_else(|| needs_lang(interface))?;
        let command = self.provider_command(lang)?;
        let executable = process::locate(&self.root, &command[0])
            .await
            .map_err(|f| config_error(format!("the provider for `{lang}`: {}", f.message)))?;
        let provider = process::file_digest(&executable)
            .await
            .map_err(|f| config_error(format!("the provider for `{lang}`: {}", f.message)))?;
        Ok(
            Digest::of(format!("config={};provider={provider}", self.config.digest).as_bytes())
                .to_string(),
        )
    }

    fn provider_command(&self, lang: &str) -> Result<&[String], RunnerError> {
        self.config
            .mutants
            .get(lang)
            .map(Vec::as_slice)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| config_error(format!("`mutants` has no command for `{lang}`")))
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
            return Err(config_error(format!(
                "`{run}` is not a name for a run's directory"
            )));
        }
        Ok(RunDirs::of(self.root.join(WORK_DIR).join(run)))
    }

    async fn mutate_run(&self, request: &MutateRequest) -> Result<MutateReport, RunnerError> {
        let (place, lang) = self
            .config
            .interface(&request.context, &request.interface)
            .ok_or_else(|| {
                config_error(format!(
                    "[[measure.layout]] has no entry for the interface `{}` of `{}`",
                    request.interface, request.context
                ))
            })?;
        let lang = lang.ok_or_else(|| needs_lang(&request.interface))?;
        let mutants_command = self.provider_command(lang)?;
        let test =
            self.config.test.as_deref().ok_or_else(|| {
                config_error("[measure] has no `test` command to mutate with".into())
            })?;
        let mut harnesses = Vec::with_capacity(request.harnesses.len());
        for h in &request.harnesses {
            let place = self
                .config
                .harness(&request.context, &h.name)
                .ok_or_else(|| {
                    config_error(format!(
                        "[[measure.layout]] has no entry for the harness `{}` of `{}`",
                        h.name, request.context
                    ))
                })?;
            harnesses.push((place.path.as_path(), h.source.as_str()));
        }
        let dirs = self.run_dirs(&request.run)?;
        let job = Job {
            runner: self,
            dirs,
            deadline: match process::deadline_in(self.config.budget) {
                Ok(d) => d,
                Err(f) => return Err(config_error(f.message)),
            },
        };
        let mut progress = Progress::default();
        let finished = job
            .steps(
                (place.path.as_path(), &request.code),
                &harnesses,
                (mutants_command, test),
                &mut progress,
            )
            .await;
        job.clean().await;
        Ok(MutateReport {
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

/// One run in its directory.
struct Job<'a> {
    runner: &'a ProcessRunner,
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
        self.runner
            .config
            .env
            .iter()
            .map(|(k, v)| (k.clone(), self.dirs.expand(v)))
            .collect()
    }

    async fn command(&self, words: &[String]) -> Result<Command, MeasureFailure> {
        let (program, args) = words
            .split_first()
            .ok_or_else(|| MeasureFailure::of(FailureKind::Io, "a command is empty"))?;
        let executable = process::locate(&self.dirs.tree, program)
            .await
            .map_err(|f| self.fail(f))?;
        let digest = process::file_digest(&executable)
            .await
            .map_err(|f| self.fail(f))?;
        Ok(Command {
            executable,
            args: args.to_vec(),
            digest,
        })
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
        let result = self
            .exec(command, env, self.runner.config.baseline_timeout)
            .await;
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

    /// What became of the code in the copy as it is now: built, then tested.
    async fn judge(
        &self,
        build: &Command,
        test: &Command,
        env: &[(String, String)],
    ) -> Result<MutantOutcome, MeasureFailure> {
        let config = &self.runner.config;
        match self.exec(build, env, config.timeout).await {
            Ok((ran, _)) if !ran.success => return Ok(MutantOutcome::Stillborn),
            Ok(_) => {}
            Err(f) if f.kind == process::FailureKind::Timeout && !self.over_budget() => {
                return Ok(MutantOutcome::Timeout);
            }
            Err(f) => return Err(self.fail(f)),
        }
        match self.exec(test, env, config.timeout).await {
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
        code: (&Path, &str),
        harnesses: &[(&Path, &str)],
        commands: (&[String], &[String]),
        out: &mut Progress,
    ) -> Result<(), MeasureFailure> {
        let (mutants_command, test_words) = commands;
        let config = &self.runner.config;
        let (root, tree, exclude) = (
            self.runner.root.clone(),
            self.dirs.tree.clone(),
            config.copy_exclude.clone(),
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
        let env = self.env();
        let build = self.command(&config.build).await?;
        let test = self.command(test_words).await?;
        self.baseline("baseline-build", &build, &env, out).await?;
        self.baseline("baseline-test", &test, &env, out).await?;

        let (asked, evidence) = provider::ask(
            &self.runner.root,
            mutants_command,
            &path.to_string_lossy(),
            code,
            config.max_mutants,
            config.seed,
            config.operators.as_deref(),
            self.deadline,
        )
        .await;
        out.evidence.push(evidence);
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
            let outcome = self.judge(&build, &test, &env).await;
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
    use crate::app::ports::measure::HarnessText;
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

    fn request(run: &str) -> MutateRequest {
        MutateRequest {
            run: run.to_string(),
            context: "ctx".to_string(),
            interface: "api".to_string(),
            code: PINNED.to_string(),
            harnesses: vec![HarnessText {
                name: "checks".to_string(),
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
        unknown.interface = "other".to_string();
        let error = runner.mutate_run(&unknown).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no entry for the interface `other`")
        );
        let mut no_harness = request("run-8");
        no_harness.harnesses[0].name = "elsewhere".to_string();
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

    #[tokio::test]
    async fn the_settings_follow_the_table_and_the_provider_program() {
        let project = Project::new(TABLE);
        let one = project.runner().settings_of("ctx", "api").await.unwrap();
        assert_eq!(
            one,
            project.runner().settings_of("ctx", "api").await.unwrap()
        );
        // another table is other settings
        project.table(&TABLE.replace("timeout = 5", "timeout = 6"));
        let two = project.runner().settings_of("ctx", "api").await.unwrap();
        assert_ne!(one, two);
        // so is another provider program under the same table
        std::fs::write(
            project.0.join("provider.sh"),
            format!("{PROVIDER}# changed\n"),
        )
        .unwrap();
        let three = project.runner().settings_of("ctx", "api").await.unwrap();
        assert_ne!(two, three);
        // an interface with no layout has none
        assert!(project.runner().settings_of("ctx", "other").await.is_err());
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
        assert_eq!(runner.language("ctx", "api"), Ok("text".to_string()));
        assert_eq!(runner.language("ctx", "bare"), Err(ItemsError::NoLanguage));
        assert_eq!(
            runner.language("ctx", "nowhere"),
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
        r.interface = "api".to_string();
        let error = project.runner().mutate_run(&r).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`mutants` has no command for `text`"),
            "{error}"
        );
        let error = project
            .runner()
            .settings_of("ctx", "api")
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
