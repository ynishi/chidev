//! The port of the measure runner: what the use case hands over to have code measured, and what comes back. How the
//! code is copied, built and run, and how a provider is asked for mutants, is the adapter's. A run reports what it
//! observed as values of the domain (a mutant and its outcome, the evidence of a command); the use case stamps it
//! and keeps it.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::domain::vo::measure::{MeasureFailure, MutantResult, StepEvidence};

/// A future that is boxed so that the port can be used as a trait object: the use case holds one runner for the
/// life of the program, chosen by the program that starts it.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The source of one Harness, with the name the project's configuration lays it out by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessText {
    /// The Harness's name in its Context.
    pub name: String,
    /// Its code, as the model holds it now.
    pub source: String,
}

/// One request to mutate the code of an Interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutateRequest {
    /// An id of this run, unique, for the directory the run works in.
    pub run: String,
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub interface: String,
    /// The code the Pin accepted, whole.
    pub code: String,
    /// The Harnesses that check the Interface, as the model holds them now.
    pub harnesses: Vec<HarnessText>,
}

/// What a run observed. A run that failed carries the failure and what it got through before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutateReport {
    /// How many places the provider found to change.
    pub sites: u32,
    /// The mutants that were run, with their outcomes.
    pub mutants: Vec<MutantResult>,
    /// Why the run did not finish; none when it did.
    pub failure: Option<MeasureFailure>,
    /// What the commands did.
    pub evidence: Vec<StepEvidence>,
}

/// Why a run could not even be set up: nothing was run, so nothing is kept.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunnerError {
    /// The program was started without a measure configuration.
    #[error("no measure is configured; add a [measure] table to checks.toml")]
    Unconfigured,
    /// The configuration cannot measure this: no layout for the Interface or a Harness, no provider for its
    /// language, a provider that cannot be found.
    #[error("{0}")]
    Config(String),
}

/// Consumer-owned port to run a measure. An implementation copies the project, never touches the project's own
/// files, bounds what it runs by deadlines, and removes what it made.
pub trait Runner: fmt::Debug + Send + Sync {
    /// A digest of everything the configuration contributes to a run of `interface` of `context` (commands, layout,
    /// provider and its executable): a measurement made with another one is made under other settings.
    fn settings<'a>(
        &'a self,
        context: &'a str,
        interface: &'a str,
    ) -> BoxFuture<'a, Result<String, RunnerError>>;

    /// Mutates the code of an Interface and runs the Harnesses against each mutant.
    fn mutate<'a>(
        &'a self,
        request: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>>;
}

/// The runner of an application that has no measure configured.
#[derive(Debug)]
pub struct NoRunner;

impl Runner for NoRunner {
    fn settings<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn mutate<'a>(
        &'a self,
        _: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }
}
