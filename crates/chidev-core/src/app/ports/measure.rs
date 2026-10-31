//! The port of the measure runner: what the use case hands over to have code measured, and what comes back. How the
//! code is copied, built and run, and how a provider is asked for mutants, is the adapter's. A run reports what it
//! observed as values of the domain (a mutant and its outcome, the evidence of a command); the use case stamps it
//! and keeps it.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::domain::vo::example::{Example, ExampleName, Verdict};
use crate::domain::vo::measure::{MeasureFailure, MutantResult, SettingsDigest, StepEvidence};
use crate::domain::vo::name::{AssetName, ContextName};
use crate::domain::vo::sim::{CaseRun, CitedClause, RegressName, Rerun};

/// A future that is boxed so that the port can be used as a trait object: the use case holds one runner for the
/// life of the program, chosen by the program that starts it.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The source of one Harness, with the name the project's configuration lays it out by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessText {
    /// The Harness's name in its Context.
    pub name: AssetName,
    /// Its code, as the model holds it now.
    pub source: String,
}

/// One request to mutate the code of an Interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutateRequest {
    /// An id of this run, unique, for the directory the run works in.
    pub run: String,
    /// The Context's name.
    pub context: ContextName,
    /// The Interface's name.
    pub interface: AssetName,
    /// The code the Pin accepted, whole.
    pub code: String,
    /// The Harnesses that check the Interface, as the model holds them now.
    pub harnesses: Vec<HarnessText>,
}

/// What a run observed. A run that failed carries the failure and what it got through before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutateReport {
    /// The digest of the settings the run resolved and ran under. The measurement is stamped with this one, and not
    /// with a digest asked for before the run, which another resolution could have differed from.
    pub settings: SettingsDigest,
    /// How many places the provider found to change.
    pub sites: u32,
    /// How many mutants the provider gave to run (at most `max_mutants` of the sites); the mutants below are fewer
    /// only when the run failed.
    pub planned: u32,
    /// The mutants that were run, with their outcomes.
    pub mutants: Vec<MutantResult>,
    /// Why the run did not finish; none when it did.
    pub failure: Option<MeasureFailure>,
    /// What the commands did.
    pub evidence: Vec<StepEvidence>,
}

/// The code a Pin accepted, with the name of the Interface it is the code of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceText {
    /// The Interface's name in its Context.
    pub name: AssetName,
    /// The code the Pin accepted, whole.
    pub code: String,
}

/// One request to simulate a sim Harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulateRequest {
    /// An id of this run, unique, for the directory the run works in.
    pub run: String,
    /// The Context's name.
    pub context: ContextName,
    /// The sim Harness's name.
    pub harness: AssetName,
    /// The script, as the model holds it now.
    pub script: String,
    /// The code the Pins accepted for the Interfaces the Harness builds.
    pub interfaces: Vec<InterfaceText>,
    /// The Clauses the sweep Case cites, each by the name a script writes it (`<contract>:<label>`) and its id: a
    /// report that names another is a protocol failure, and a name in the report is resolved to the id here.
    pub clauses: Vec<CitedClause>,
    /// The regress cases of the Harness, in the order they are to be run.
    pub regress: Vec<RegressName>,
}

/// What a simulation observed. A run that failed carries the failure and what it got through before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulateReport {
    /// The digest of the settings the run resolved and ran under; the measurement is stamped with it.
    pub settings: SettingsDigest,
    /// Each case that was run, in order: the seeds, then the regress cases.
    pub cases: Vec<CaseRun>,
    /// The sampled cases, run a second time in a fresh copy, and how they compared.
    pub reruns: Vec<Rerun>,
    /// Why the run did not finish; none when it did.
    pub failure: Option<MeasureFailure>,
    /// What the commands did.
    pub evidence: Vec<StepEvidence>,
}

/// One request to run the examples of a Harness of examples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamplesRequest {
    /// An id of this run, unique, for the directory the run works in.
    pub run: String,
    /// The Context's name.
    pub context: ContextName,
    /// The Harness of examples' name.
    pub harness: AssetName,
    /// The code the Pins accepted for the Interfaces the Harness builds.
    pub interfaces: Vec<InterfaceText>,
    /// The examples, each by the name of its Case, in the order they are to be run, as the model holds them now.
    /// The report names an example by the same name, and the use case resolves it back to the Case.
    pub examples: Vec<(ExampleName, Example)>,
}

/// What a run of examples observed. A run that failed carries the failure and what it got through before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamplesReport {
    /// The digest of the settings the run resolved and ran under; the measurement is stamped with it.
    pub settings: SettingsDigest,
    /// The verdict of each example that was run, by its name, in the order they were run.
    pub examples: Vec<(ExampleName, Verdict)>,
    /// Why the run did not finish; none when it did.
    pub failure: Option<MeasureFailure>,
    /// What the commands did.
    pub evidence: Vec<StepEvidence>,
}

/// What a layout entry places: the code of an Interface, or the source of a Harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPlace {
    /// An Interface's code.
    Interface,
    /// A Harness's source.
    Harness,
}

impl fmt::Display for LayoutPlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LayoutPlace::Interface => "interface",
            LayoutPlace::Harness => "harness",
        })
    }
}

/// Which command of a run a program belongs to, for a message about the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRole {
    /// The command that builds the copy.
    Build,
    /// The command that runs the tests.
    Test,
    /// The provider that makes the mutants of a language.
    Provider {
        /// The language of the Interface the provider is for.
        lang: String,
    },
    /// The interpreter a sim script runs under.
    Interpreter,
    /// The program under test of a simulation: the one its script drives.
    SimProgram,
    /// The program under test of a run of examples: the one the examples are run against.
    ExamplesProgram,
}

impl fmt::Display for CommandRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandRole::Build => f.write_str("the build command"),
            CommandRole::Test => f.write_str("the test command"),
            CommandRole::Provider { lang } => write!(f, "the provider for `{lang}`"),
            CommandRole::Interpreter => f.write_str("the interpreter"),
            CommandRole::SimProgram => f.write_str("the program of the simulation"),
            CommandRole::ExamplesProgram => f.write_str("the program of the examples"),
        }
    }
}

/// Why a run could not even be set up, or why the settings of one cannot be read: nothing was run, so nothing is
/// kept. Each way is its own variant, for a caller has to tell a project with no `[measure]` from one whose table
/// cannot measure this.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunnerError {
    /// The program was started without a measure configuration.
    #[error("no measure is configured; add a [measure] table to the project's settings")]
    Unconfigured,
    /// `[[measure.layout]]` has no entry for the Interface or the Harness.
    #[error("[[measure.layout]] has no entry for the {place} `{name}` of `{context}`")]
    NoLayout {
        /// Whether it is an Interface or a Harness that is not laid out.
        place: LayoutPlace,
        /// The Context it is of.
        context: ContextName,
        /// Its name.
        name: AssetName,
    },
    /// The layout of the Interface has no `lang`, which picks the provider that mutates its code.
    #[error("the layout of interface `{interface}` has no `lang`, which mutating its code needs")]
    NoLang {
        /// The Interface.
        interface: AssetName,
    },
    /// `mutants` has no command for the language.
    #[error("`mutants` has no command for `{lang}`")]
    NoProvider {
        /// The language of the Interface.
        lang: String,
    },
    /// `[measure]` has no `test` command, which a mutation run needs.
    #[error("[measure] has no `test` command to mutate with")]
    NoTestCommand,
    /// `[measure.sim.<context>]` is not there.
    #[error("[measure.sim.{context}] is not configured; it gives the interpreter and the program")]
    NoSimConfig {
        /// The Context that has no table.
        context: ContextName,
    },
    /// `[measure.examples.<context>]` is not there.
    #[error(
        "[measure.examples.{context}] is not configured; it gives the program the examples are run against"
    )]
    NoExamplesConfig {
        /// The Context that has no table.
        context: ContextName,
    },
    /// The program of a command cannot be found.
    #[error("{what}: {reason}")]
    Locate {
        /// Whose program it is.
        what: CommandRole,
        /// Why it is not found.
        reason: String,
    },
    /// The program of a command is found, but its digest cannot be read.
    #[error("{what}: {reason}")]
    Fingerprint {
        /// Whose program it is.
        what: CommandRole,
        /// Why its bytes cannot be read.
        reason: String,
    },
    /// The name of a run is not one for a directory: it could lead out of the directory the runs work in.
    #[error("`{0}` is not a name for a run's directory")]
    BadRunName(String),
    /// The settings cannot be written down to be digested.
    #[error("the settings cannot be digested: {0}")]
    Digest(String),
    /// A time limit of the configuration is more than the clock can count.
    #[error("{0}")]
    Deadline(String),
}

/// One subject whose settings are asked about, with the names the single-subject asks of [`Runner`] take: what
/// [`Runner::settings`], [`Runner::sim_settings`] and [`Runner::examples_settings`] are each asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsTarget {
    /// A mutation run of `interface` of `context` that runs the tests `harnesses` ([`Runner::settings`]).
    Mutation {
        /// The Context's name.
        context: ContextName,
        /// The Interface's name.
        interface: AssetName,
        /// The Harnesses the run runs as tests.
        harnesses: Vec<AssetName>,
    },
    /// A simulation of the sim Harness `harness` of `context` that builds `builds` ([`Runner::sim_settings`]).
    Simulation {
        /// The Context's name.
        context: ContextName,
        /// The sim Harness's name.
        harness: AssetName,
        /// The Interfaces it builds.
        builds: Vec<AssetName>,
    },
    /// A run of the examples of the Harness `harness` of `context` that builds `builds`
    /// ([`Runner::examples_settings`]).
    Examples {
        /// The Context's name.
        context: ContextName,
        /// The Harness of examples' name.
        harness: AssetName,
        /// The Interfaces it builds.
        builds: Vec<AssetName>,
    },
}

/// What a runner said of each [`SettingsTarget`] it was asked about, each beside the target it is of.
pub type SettingsSaid = Vec<(SettingsTarget, Result<SettingsDigest, RunnerError>)>;

/// Consumer-owned port to run a measure. An implementation copies the project, never touches the project's own
/// files, bounds what it runs by deadlines, and removes what it made.
pub trait Runner: fmt::Debug + Send + Sync {
    /// The digests of the settings of many subjects at once, each paired with the target it is of (never by
    /// position). Each digest is what the single ask of the target's kind gives for it ([`Runner::settings`],
    /// [`Runner::sim_settings`], [`Runner::examples_settings`]); asking together lets an implementation reuse what
    /// the targets share within the one call, and nothing outlives it. The default asks one target after another.
    fn settings_of<'a>(&'a self, targets: &'a [SettingsTarget]) -> BoxFuture<'a, SettingsSaid> {
        Box::pin(async move {
            let mut said = Vec::with_capacity(targets.len());
            for target in targets {
                let digest = match target {
                    SettingsTarget::Mutation {
                        context,
                        interface,
                        harnesses,
                    } => self.settings(context, interface, harnesses).await,
                    SettingsTarget::Simulation {
                        context,
                        harness,
                        builds,
                    } => self.sim_settings(context, harness, builds).await,
                    SettingsTarget::Examples {
                        context,
                        harness,
                        builds,
                    } => self.examples_settings(context, harness, builds).await,
                };
                said.push((target.clone(), digest));
            }
            said
        })
    }

    /// The digest of what a mutation run of `interface` of `context` that runs the tests `harnesses` reads of the
    /// configuration: a measurement made with another digest is made under other settings. It covers the settings
    /// that run reads and no others. It says what the settings are now; the stamp of a run is the digest in its
    /// [`MutateReport`].
    fn settings<'a>(
        &'a self,
        context: &'a ContextName,
        interface: &'a AssetName,
        harnesses: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>>;

    /// Mutates the code of an Interface and runs the Harnesses against each mutant.
    fn mutate<'a>(
        &'a self,
        request: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>>;

    /// The digest of what a simulation of the sim Harness `harness` of `context` reads of the configuration, if
    /// the Interfaces `builds` are the ones it builds. An error when the configuration cannot simulate it.
    fn sim_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>>;

    /// Builds the program from the pinned code and runs the script for each case, then again for a sample of the
    /// cases in a fresh copy, to see that the same case gives the same schedule and trace.
    fn simulate<'a>(
        &'a self,
        request: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>>;

    /// The digest of what a run of the examples of the Harness `harness` of `context` reads of the configuration,
    /// if the Interfaces `builds` are the ones it builds. An error when the configuration cannot run it.
    fn examples_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>>;

    /// Builds the program from the pinned code and runs each example against it, one after another, each in the
    /// copy as the build left it. The verdict of an example is the domain's judge of what the program answered.
    fn run_examples<'a>(
        &'a self,
        request: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>>;
}

/// The runner of an application that has no measure configured.
#[derive(Debug)]
pub struct NoRunner;

impl Runner for NoRunner {
    fn settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn mutate<'a>(
        &'a self,
        _: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn sim_settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn simulate<'a>(
        &'a self,
        _: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn examples_settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn run_examples<'a>(
        &'a self,
        _: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_application_with_no_measure_configured_can_read_no_settings() {
        let (context, name) = (
            ContextName::try_from("core").unwrap(),
            AssetName::try_from("api").unwrap(),
        );
        let error = NoRunner.settings(&context, &name, &[]).await.unwrap_err();
        assert_eq!(error, RunnerError::Unconfigured);
        assert_eq!(
            error.to_string(),
            "no measure is configured; add a [measure] table to the project's settings"
        );
        assert_eq!(
            NoRunner
                .sim_settings(&context, &name, &[])
                .await
                .unwrap_err(),
            RunnerError::Unconfigured
        );
        assert_eq!(
            NoRunner
                .examples_settings(&context, &name, &[])
                .await
                .unwrap_err(),
            RunnerError::Unconfigured
        );
        let targets = [
            SettingsTarget::Mutation {
                context: context.clone(),
                interface: name.clone(),
                harnesses: Vec::new(),
            },
            SettingsTarget::Simulation {
                context: context.clone(),
                harness: name.clone(),
                builds: Vec::new(),
            },
            SettingsTarget::Examples {
                context: context.clone(),
                harness: name.clone(),
                builds: Vec::new(),
            },
        ];
        let said = NoRunner.settings_of(&targets).await;
        assert_eq!(said.len(), targets.len());
        for (target, said) in targets.iter().zip(said) {
            assert_eq!(said, (target.clone(), Err(RunnerError::Unconfigured)));
        }
        assert!(NoRunner.settings_of(&[]).await.is_empty());
        let request = ExamplesRequest {
            run: "run".to_string(),
            context,
            harness: name,
            interfaces: Vec::new(),
            examples: Vec::new(),
        };
        assert_eq!(
            NoRunner.run_examples(&request).await.unwrap_err(),
            RunnerError::Unconfigured
        );
    }

    #[test]
    fn the_role_of_a_command_is_named_in_a_message_and_the_provider_by_its_language() {
        assert_eq!(CommandRole::Build.to_string(), "the build command");
        assert_eq!(CommandRole::Test.to_string(), "the test command");
        assert_eq!(CommandRole::Interpreter.to_string(), "the interpreter");
        assert_eq!(
            CommandRole::SimProgram.to_string(),
            "the program of the simulation"
        );
        assert_eq!(
            CommandRole::ExamplesProgram.to_string(),
            "the program of the examples"
        );
        assert_eq!(
            CommandRole::Provider {
                lang: "rust".to_string()
            }
            .to_string(),
            "the provider for `rust`"
        );
    }
}
