//! Running the examples of a Harness of examples.
//!
//! chidev copies the project (as a mutation run and a simulation do), writes the code the Pins of the Interfaces the
//! Harness builds accepted over the copy, and runs the project's build command. It then starts the configured
//! `program` once per example:
//!
//! ```text
//! <program words...> <the arguments of the example...>
//! ```
//!
//! with the example's input on standard input, the copy as the working directory and the environment of the table.
//! `{tree}` in a word of the program is the root of the copy and `{run}` the directory of the run, the first word
//! included. The program is found in the copy as the build left it. Whether the settings hold its file depends on
//! where the first word starts it from ([`super::settings::ProgramStart`]): a file of the host is digested, and one in
//! the copy is not. The copy is asked, as it is made, whether the first word of a program of the host still resolves
//! (a relative entry of `PATH` being the copy's) to the file the settings have, and a copy in which it does not fails
//! the run (`settings_moved`) before anything is started; the copy's own program has no digest to be held to.
//!
//! One example is run and judged as `chidev accept` runs and judges one ([`crate::infra::accept`]), so the two cannot
//! differ on what a program that ended answered. An example that gets no answer from the program (it cannot be
//! started, it does not end within `timeout`, it writes too much, a signal ends it) is a failed example with that one
//! reason, and the run goes on. The run itself fails (and keeps the examples it got through) when the program does
//! not build or the budget is used up.

use std::path::Path;

use super::settings::{ExamplesSettings, ProgramUnderTest, find};
use super::{Command, Job, ProcessRunner};
use crate::app::ports::measure::{
    ExamplesReport, ExamplesRequest, InterfaceText, LayoutPlace, RunnerError,
};
use crate::domain::vo::example::{ExampleName, Verdict, Why};
use crate::domain::vo::measure::{MeasureFailure, SettingsDigest, StepEvidence};
use crate::domain::vo::name::{AssetName, ContextName};
use crate::infra::accept::{self, Example, Resolved};
use crate::infra::process;

/// What a run of examples has observed so far.
#[derive(Debug, Default)]
struct ExamplesProgress {
    examples: Vec<(ExampleName, Verdict)>,
    evidence: Vec<StepEvidence>,
}

impl ProcessRunner {
    /// The digest of the settings of a run of the examples of a Harness of `context` that builds the Interfaces
    /// `builds`.
    pub(super) async fn examples_settings_of(
        &self,
        context: &ContextName,
        _harness: &AssetName,
        builds: &[AssetName],
    ) -> Result<SettingsDigest, RunnerError> {
        // The Harness is asked about by the port but the run reads nothing of it: the examples are the request's.
        ExamplesSettings::resolve(&self.config, &self.root, context, builds)
            .await?
            .digest()
    }

    /// Runs the examples `request` asks for.
    pub(super) async fn examples_run(
        &self,
        request: &ExamplesRequest,
    ) -> Result<ExamplesReport, RunnerError> {
        let builds: Vec<AssetName> = request.interfaces.iter().map(|i| i.name.clone()).collect();
        let settings =
            ExamplesSettings::resolve(&self.config, &self.root, &request.context, &builds).await?;
        let digest = settings.digest()?;
        let mut interfaces = Vec::with_capacity(request.interfaces.len());
        for i in &request.interfaces {
            let path = settings
                .build_path(&i.name)
                .ok_or_else(|| RunnerError::NoLayout {
                    place: LayoutPlace::Interface,
                    context: request.context.clone(),
                    name: i.name.clone(),
                })?;
            interfaces.push((path, i));
        }
        let dirs = self.run_dirs(&request.run)?;
        let deadline =
            process::deadline_in(settings.budget).map_err(|f| RunnerError::Deadline(f.message))?;
        let job = Job {
            root: &self.root,
            copy: &settings.copy,
            dirs,
            deadline,
        };
        let mut progress = ExamplesProgress::default();
        let finished = job
            .examples_steps(&settings, &interfaces, request, &mut progress)
            .await;
        job.clean().await;
        Ok(ExamplesReport {
            settings: digest,
            examples: progress.examples,
            failure: finished.err(),
            evidence: progress.evidence,
        })
    }
}

impl Job<'_> {
    /// The whole run: build, then every example in the order of the request.
    async fn examples_steps(
        &self,
        settings: &ExamplesSettings,
        interfaces: &[(&Path, &InterfaceText)],
        request: &ExamplesRequest,
        out: &mut ExamplesProgress,
    ) -> Result<(), MeasureFailure> {
        let written: Vec<(&Path, &str)> = interfaces
            .iter()
            .map(|(path, i)| (*path, i.code.as_str()))
            .collect();
        // The program under test is held to the settings when the copy is made if its first word starts from the host;
        // one in the copy has no digest in them (`ProgramStart`).
        self.prepare(
            "examples",
            &written,
            &[],
            &settings.program,
            &mut out.evidence,
        )
        .await?;
        // The program is the file the copy has now, which the build may have written.
        let program = self.program_command(&settings.program).await?;
        let fixed = program.args.clone();
        let env = self.env();
        for (name, example) in &request.examples {
            if self.over_budget() {
                return Err(self.budget_used_up());
            }
            let held = Example::from((name.clone(), example));
            // an example is not given more time than the run has left
            let left = self
                .deadline
                .saturating_duration_since(tokio::time::Instant::now());
            let timeout = settings.timeout.min(left);
            let verdict = accept::run_resolved(
                &held,
                &Resolved {
                    executable: &program.executable,
                    fixed: &fixed,
                    env: &env,
                    cwd: &self.dirs.tree,
                },
                timeout,
            )
            .await;
            // An example that was cut short by the budget and not by its own limit says nothing of the program:
            // the run ends, and the example is not recorded as one the program failed.
            let cut = timeout < settings.timeout && gave_no_answer(&verdict);
            if cut && self.over_budget() {
                return Err(self.budget_used_up());
            }
            out.examples.push((name.clone(), verdict));
        }
        Ok(())
    }

    /// The command that starts the program under test `program` in the copy: its words made the places of the run,
    /// and the file the first names there now, found by the one rule of [`find`]. A program that is not there when
    /// the run starts it is the run's failure.
    async fn program_command(&self, program: &ProgramUnderTest) -> Result<Command, MeasureFailure> {
        let found = find(&self.dirs.tree, &self.dirs.expand(program.program()))
            .await
            .map_err(|e| self.fail(e.failure()))?;
        Ok(Command {
            executable: found.executable,
            args: program.args().iter().map(|w| self.dirs.expand(w)).collect(),
            digest: found.digest,
        })
    }

    /// The failure of a run that has used up its time.
    fn budget_used_up(&self) -> MeasureFailure {
        super::from_process(
            process::Failure {
                kind: process::FailureKind::Timeout,
                message: String::new(),
            },
            true,
        )
    }
}

/// Whether the verdict is that the program gave no answer: it did not end, or could not be started.
fn gave_no_answer(verdict: &Verdict) -> bool {
    match verdict {
        Verdict::Ok => false,
        Verdict::Fail(reasons) => reasons.iter().any(|why| matches!(why, Why::Abnormal(_))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_verdict_with_no_answer_behind_it_can_be_one_the_budget_cut() {
        assert!(!gave_no_answer(&Verdict::Ok));
        assert!(!gave_no_answer(&Verdict::fail(Why::Exit {
            expected: 0,
            got: 1
        })));
        assert!(gave_no_answer(&Verdict::fail(Why::Abnormal(
            "did not end within 1 s".to_string()
        ))));
    }
}
