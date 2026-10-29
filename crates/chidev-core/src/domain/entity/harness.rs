//! The Harness: the cases that check a Context, as code or as examples, each citing what it checks.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::example::{Example, ExampleName, ExamplePart, HARNESS_LIMIT, TEXT_LIMIT};
use crate::domain::vo::harness_kind::{CodeKind, HarnessKind};
use crate::domain::vo::id::{CaseId, ClauseId, ContextId, HarnessId, InterfaceId};
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::sim::{CaseRole, RegressName, role_of};
use crate::domain::vo::source::Source;

/// What a Case holds, by the kind of Harness it is in: the lines of the code it is on (a test, a case of a
/// simulation), or an example, which is data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseBody {
    /// The lines of the Harness's code the Case is on.
    Lines(LineRange),
    /// An example of what a program answers.
    Example(Example),
}

/// One case of a Harness (a test, or an example). It is known by its id, so a record of it survives a rename; its
/// revision says whether its body or what it cites changed. Where a test is in the code (its lines) is shown but is
/// not its content: moving it changes no revision. An example is nothing but its content: a change of one of its
/// texts is a new revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    id: CaseId,
    revision: Revision,
    name: CaseName,
    cites: CaseCites,
    body: CaseBody,
}

impl Case {
    /// Its id.
    pub fn id(&self) -> CaseId {
        self.id
    }

    /// How many times its body or its citations have changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its name.
    pub fn name(&self) -> &CaseName {
        &self.name
    }

    /// What it cites.
    pub fn cites(&self) -> &CaseCites {
        &self.cites
    }

    /// What it holds.
    pub fn body(&self) -> &CaseBody {
        &self.body
    }

    /// The lines of the code it is on; none for an example.
    pub fn lines(&self) -> Option<LineRange> {
        match &self.body {
            CaseBody::Lines(lines) => Some(*lines),
            CaseBody::Example(_) => None,
        }
    }

    /// The example it holds; none for a Case of code.
    pub fn example(&self) -> Option<&Example> {
        match &self.body {
            CaseBody::Lines(_) => None,
            CaseBody::Example(example) => Some(example),
        }
    }
}

/// Why code and a list of Cases are not a Harness. The Harness is not made or changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HarnessError {
    /// Two Cases have one id.
    #[error("the case id {0} is given twice")]
    DuplicateId(CaseId),
    /// The lines of a Case are not inside the code.
    #[error("the case {id} is on lines {first}..={last}, past the {total} lines of the code")]
    RangeOutside {
        /// The Case.
        id: CaseId,
        /// The first line it was given.
        first: usize,
        /// The last line it was given.
        last: usize,
        /// How many lines the code has.
        total: usize,
    },
    /// A Case of a Harness of tests cites no Item; a test checks something the Interface shows.
    #[error("the case {0} of a harness of tests cites no item")]
    NoItem(CaseId),
    /// A Case of a sim Harness cites an Item. A simulation checks Clauses; the Interfaces whose pinned code it
    /// builds are the ones the Harness names in `builds`, not ones its Cases cite.
    #[error(
        "the case {0} of a sim harness cites an item; a sim case cites clauses only (name the interfaces to build in `builds`)"
    )]
    SimCitesItem(CaseId),
    /// A Harness of tests names Interfaces to build; only a sim Harness builds the program it drives.
    #[error(
        "the harness {0} of tests has no `builds`; only a sim harness or a harness of examples builds the interfaces it runs"
    )]
    BuildsOfTests(HarnessId),
    /// A Case of a Harness of examples cites an Item. An example shows what a Clause promises; the Interfaces whose
    /// pinned code the program is built from are the ones the Harness names in `builds`.
    #[error(
        "the case {0} of a harness of examples cites an item; an example case cites clauses only (name the interfaces to build in `builds`)"
    )]
    ExampleCitesItem(CaseId),
    /// A Case of a Harness of examples cites no Clause, so nothing says what the example is an example of.
    #[error(
        "the case {0} of a harness of examples cites no clause; an example case cites the clauses it shows"
    )]
    ExampleCitesNone(CaseId),
    /// The name of a Case of a Harness of examples is not one an example can have, or another Case has it.
    #[error("the case {case} of a harness of examples is named wrongly: {}", .fault.word())]
    ExampleName {
        /// The Case.
        case: CaseId,
        /// What is wrong with the name.
        fault: ExampleNameFault,
    },
    /// A Harness of examples builds no Interface, so it has no program to run.
    #[error("the harness {0} of examples builds no interface, so it has no program to run")]
    ExamplesBuildNone(HarnessId),
    /// One text of an example is longer than a put takes.
    #[error("the {} of the example case {case} is {size} bytes; a text is at most {limit}", .part.word())]
    ExampleTooLarge {
        /// The Case.
        case: CaseId,
        /// The text that is too long.
        part: ExamplePart,
        /// How many bytes it has.
        size: usize,
        /// How many a put takes.
        limit: usize,
    },
    /// The texts of the examples of a Harness together are longer than a put takes.
    #[error(
        "the examples of the harness {harness} are {size} bytes together; a harness takes at most {limit}"
    )]
    ExamplesTooLarge {
        /// The Harness.
        harness: HarnessId,
        /// How many bytes the texts have together.
        size: usize,
        /// How many a put takes.
        limit: usize,
    },
    /// Code is put to a Harness that holds examples, without saying it is to become a Harness of code. A Harness
    /// does not change its kind unasked.
    #[error(
        "the harness {harness} holds examples; put its examples with kind `examples`, or say kind `tests` or `sim` to make it a harness of code"
    )]
    KindContent {
        /// The Harness.
        harness: HarnessId,
    },
}

/// What is wrong with the name of a Case of a Harness of examples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExampleNameFault {
    /// The name is not one an example has: it is empty, or holds `/`, NUL or a line feed.
    NotAName,
    /// Another Case of the Harness has the name.
    NamedTwice,
}

impl ExampleNameFault {
    /// The word for it, as a violation carries it.
    pub fn word(self) -> &'static str {
        match self {
            ExampleNameFault::NotAName => "not-a-name",
            ExampleNameFault::NamedTwice => "named-twice",
        }
    }
}

/// What is wrong with the name of a Case of a sim Harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseNameFault {
    /// The name is neither `sweep` nor `regress-<name>` (a name of a-z, 0-9 and `-`).
    NeitherSweepNorRegress,
    /// Another Case of the Harness has the regress name.
    RegressNamedTwice,
}

impl CaseNameFault {
    /// The word for it, as a violation carries it.
    pub fn word(self) -> &'static str {
        match self {
            CaseNameFault::NeitherSweepNorRegress => "neither-sweep-nor-regress",
            CaseNameFault::RegressNamedTwice => "regress-named-twice",
        }
    }
}

/// A way a sim Harness is not the shape the protocol runs. It holds ids and words, never a sentence: the rule it
/// breaks is given by its violation (see `domain::violation::map`), and the names are the application's.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SimShapeProblem {
    /// A Context has more than one sim Harness, so a Clause would be checked by several scripts.
    #[error("the context {context} has {} sim harnesses", .harnesses.len())]
    OnePerContext {
        /// The Context.
        context: ContextId,
        /// Its sim Harnesses, in the order of their ids.
        harnesses: Vec<HarnessId>,
    },
    /// A sim Harness has no Case named `sweep`, or more than one.
    #[error("the sim harness {harness} has {count} cases named `sweep`")]
    SweepOnce {
        /// The Harness.
        harness: HarnessId,
        /// How many Cases are named `sweep`.
        count: usize,
        /// One of the Cases beyond the first; none when there is no sweep at all.
        extra: Option<CaseId>,
    },
    /// A Case of a sim Harness is not named `sweep` or `regress-<name>`, or repeats a regress name.
    #[error("the case {case} of the sim harness {harness} is named wrongly: {}", .fault.word())]
    CaseName {
        /// The Harness.
        harness: HarnessId,
        /// The Case.
        case: CaseId,
        /// What is wrong with the name.
        fault: CaseNameFault,
    },
    /// A sim Case cites no Clause.
    #[error("the case {case} of the sim harness {harness} cites no clause")]
    CaseCitesNone {
        /// The Harness.
        harness: HarnessId,
        /// The Case.
        case: CaseId,
    },
    /// A sim Case cites a Clause that no Contract of the Harness's Context has.
    #[error(
        "the case {case} of the sim harness {harness} cites the clause {clause}, which is in no contract of its context"
    )]
    ClauseForeign {
        /// The Harness.
        harness: HarnessId,
        /// The Case.
        case: CaseId,
        /// The Clause.
        clause: ClauseId,
    },
    /// A sim Harness builds no Interface, so it has no program to drive.
    #[error("the sim harness {harness} builds no interface")]
    BuildsNone {
        /// The Harness.
        harness: HarnessId,
    },
}

/// The cases that check a Context. A Harness is code or examples, and holds the one it is, never both.
///
/// The code is the content (it is what runs); the Cases are the tests in it, found by the caller, who knows the
/// language, with what each cites. Whether what a Case cites exists is a rule of the Context that owns the Harness,
/// not of the Harness.
///
/// A Harness of tests cites Items (every Case at least one) and Clauses, and the Pin of an Interface stands on the
/// Harnesses whose Cases cite its Items. A sim Harness is a script that drives a program: its Cases cite Clauses
/// only (they are the names the protocol runs the script under, and the Clauses the script checks), and the
/// program is built from the pinned code of the Interfaces in `builds`. Naming an Interface to build is not citing
/// it: it puts the Harness on no Pin's basis and asks no Item of it. Only a sim Harness or a Harness of examples
/// has `builds`.
///
/// A Harness of examples has no code: each Case is an example, data a program is judged by, named, citing the
/// Clauses it shows (at least one) and no Item. What the examples say is the content, so a change of one text is a
/// new revision of its Case and of the Harness, which every record that stands on the Harness sees.
///
/// The content is one private value and every Case is built from the variant's own input, so no Harness has Cases
/// that disagree with what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Harness {
    id: HarnessId,
    revision: Revision,
    builds: BTreeSet<InterfaceId>,
    content: Content,
    cases: Vec<Case>,
}

/// What a Harness holds: code of a kind, or examples (which are in its Cases).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    Code { kind: CodeKind, source: Source },
    Examples,
}

impl Content {
    fn kind(&self) -> HarnessKind {
        match self {
            Content::Code { kind, .. } => HarnessKind::from(*kind),
            Content::Examples => HarnessKind::Examples,
        }
    }

    fn source(&self) -> Option<&Source> {
        match self {
            Content::Code { source, .. } => Some(source),
            Content::Examples => None,
        }
    }
}

/// A Case of code as the caller found it: its id, its name, its lines and what it cites.
pub type CaseFound = (CaseId, CaseName, LineRange, CaseCites);

/// A Case of examples as the caller states it: its id, its name, its example and what it cites.
pub type ExampleFound = (CaseId, CaseName, Example, CaseCites);

/// What a put states a Harness holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessContent {
    /// Code of a kind, and the Cases in it.
    Code {
        /// The kind of the code.
        kind: CodeKind,
        /// The code.
        source: Source,
        /// The Cases in it.
        cases: Vec<CaseFound>,
    },
    /// Examples, one for each Case.
    Examples(Vec<ExampleFound>),
}

/// What a put states, before the kind of a Harness of code is settled against what the Harness is: a put of code
/// without a kind takes the kind the Harness has (tests for a new one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HarnessStated {
    /// Code, and the kind it is to have if the put says one.
    Code {
        kind: Option<CodeKind>,
        source: Source,
        cases: Vec<CaseFound>,
    },
    /// Examples.
    Examples(Vec<ExampleFound>),
}

/// A Harness's content as a store kept it: as [`HarnessContent`], with the revision of each Case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoredContent {
    /// Code of a kind, and the Cases in it.
    Code {
        kind: CodeKind,
        source: Source,
        cases: Vec<(CaseId, Revision, CaseName, LineRange, CaseCites)>,
    },
    /// Examples, one for each Case.
    Examples(Vec<(CaseId, Revision, CaseName, Example, CaseCites)>),
}

impl StoredContent {
    /// The content, and the revision of each Case in the order of the Cases.
    fn split(self) -> (HarnessContent, Vec<Revision>) {
        match self {
            StoredContent::Code {
                kind,
                source,
                cases,
            } => {
                let (revisions, cases) = cases
                    .into_iter()
                    .map(|(id, revision, name, lines, cites)| (revision, (id, name, lines, cites)))
                    .unzip();
                (
                    HarnessContent::Code {
                        kind,
                        source,
                        cases,
                    },
                    revisions,
                )
            }
            StoredContent::Examples(cases) => {
                let (revisions, cases) = cases
                    .into_iter()
                    .map(|(id, revision, name, example, cites)| {
                        (revision, (id, name, example, cites))
                    })
                    .unzip();
                (HarnessContent::Examples(cases), revisions)
            }
        }
    }
}

/// A Case before it has a revision: its id, its name, what it holds and what it cites.
type Part = (CaseId, CaseName, CaseBody, CaseCites);

impl HarnessContent {
    /// The rules of the content, but for the size of the examples (a rule of a put, not of the record): the ids of
    /// the Cases are each given once, and `builds` and the Cases are as the kind takes them.
    ///
    /// For code: every Case's lines are inside the code, a Case of tests cites an Item, a Case of a sim none, and
    /// only a sim builds. For examples: a Case cites a Clause at least and no Item, its name is one an example has
    /// and no other Case has it, and the Harness builds an Interface at least.
    fn check(&self, id: HarnessId, builds: &BTreeSet<InterfaceId>) -> Result<(), HarnessError> {
        match self {
            HarnessContent::Code {
                kind,
                source,
                cases,
            } => check_code(id, *kind, builds, source, cases),
            HarnessContent::Examples(cases) => check_examples(id, builds, cases),
        }
    }

    /// The size limits of a put: one text, and all the texts of the Harness.
    fn check_sizes(&self, id: HarnessId) -> Result<(), HarnessError> {
        let HarnessContent::Examples(cases) = self else {
            return Ok(());
        };
        let mut total = 0usize;
        for (case, _, example, _) in cases {
            for part in ExamplePart::ALL {
                let size = example.part_size(part);
                if size > TEXT_LIMIT {
                    return Err(HarnessError::ExampleTooLarge {
                        case: *case,
                        part,
                        size,
                        limit: TEXT_LIMIT,
                    });
                }
            }
            total = total.saturating_add(example.size());
        }
        if total > HARNESS_LIMIT {
            return Err(HarnessError::ExamplesTooLarge {
                harness: id,
                size: total,
                limit: HARNESS_LIMIT,
            });
        }
        Ok(())
    }

    /// What it holds, and its Cases in order.
    fn into_parts(self) -> (Content, Vec<Part>) {
        match self {
            HarnessContent::Code {
                kind,
                source,
                cases,
            } => (
                Content::Code { kind, source },
                cases
                    .into_iter()
                    .map(|(id, name, lines, cites)| (id, name, CaseBody::Lines(lines), cites))
                    .collect(),
            ),
            HarnessContent::Examples(cases) => (
                Content::Examples,
                cases
                    .into_iter()
                    .map(|(id, name, example, cites)| (id, name, CaseBody::Example(example), cites))
                    .collect(),
            ),
        }
    }
}

/// The ids of `cases` are each given once, every Case's lines are inside `source`, and `kind` takes the Cases and
/// `builds` as given: a Case of tests cites an Item, a Case of a sim none, and only a sim builds.
fn check_code(
    id: HarnessId,
    kind: CodeKind,
    builds: &BTreeSet<InterfaceId>,
    source: &Source,
    cases: &[CaseFound],
) -> Result<(), HarnessError> {
    if kind == CodeKind::Tests && !builds.is_empty() {
        return Err(HarnessError::BuildsOfTests(id));
    }
    for (k, (id, _, lines, cites)) in cases.iter().enumerate() {
        if cases[..k].iter().any(|(i, ..)| i == id) {
            return Err(HarnessError::DuplicateId(*id));
        }
        if source.lines(*lines).is_none() {
            return Err(HarnessError::RangeOutside {
                id: *id,
                first: lines.first(),
                last: lines.last(),
                total: source.line_count(),
            });
        }
        match kind {
            CodeKind::Tests if cites.items().is_empty() => {
                return Err(HarnessError::NoItem(*id));
            }
            CodeKind::Sim if !cites.items().is_empty() => {
                return Err(HarnessError::SimCitesItem(*id));
            }
            CodeKind::Tests | CodeKind::Sim => {}
        }
    }
    Ok(())
}

/// The ids of `cases` are each given once, no Case cites an Item and each cites a Clause, each name is one an
/// example has and is not another Case's, and `builds` names an Interface.
fn check_examples(
    id: HarnessId,
    builds: &BTreeSet<InterfaceId>,
    cases: &[ExampleFound],
) -> Result<(), HarnessError> {
    if builds.is_empty() {
        return Err(HarnessError::ExamplesBuildNone(id));
    }
    for (k, (case, name, _, cites)) in cases.iter().enumerate() {
        if cases[..k].iter().any(|(i, ..)| i == case) {
            return Err(HarnessError::DuplicateId(*case));
        }
        if !cites.items().is_empty() {
            return Err(HarnessError::ExampleCitesItem(*case));
        }
        if cites.clauses().is_empty() {
            return Err(HarnessError::ExampleCitesNone(*case));
        }
        if ExampleName::new(name.as_str()).is_err() {
            return Err(HarnessError::ExampleName {
                case: *case,
                fault: ExampleNameFault::NotAName,
            });
        }
        if cases[..k].iter().any(|(_, n, ..)| n == name) {
            return Err(HarnessError::ExampleName {
                case: *case,
                fault: ExampleNameFault::NamedTwice,
            });
        }
    }
    Ok(())
}

/// Whether two bodies are the same content of a Harness. Where a test is in the code is not content (the code is
/// compared as a whole), so any two ranges are the same; an example is its texts.
fn same_content(a: &CaseBody, b: &CaseBody) -> bool {
    match (a, b) {
        (CaseBody::Lines(_), CaseBody::Lines(_)) => true,
        (CaseBody::Example(a), CaseBody::Example(b)) => a == b,
        (CaseBody::Lines(_), CaseBody::Example(_)) | (CaseBody::Example(_), CaseBody::Lines(_)) => {
            false
        }
    }
}

impl Harness {
    /// A new Harness of tests at its first revision, with its Cases in the order given, each at its first revision.
    /// The ids are the caller's: the domain makes none.
    pub fn new(
        id: HarnessId,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<Harness, HarnessError> {
        Harness::of_kind(CodeKind::Tests, id, BTreeSet::new(), source, cases)
    }

    /// A new Harness of code of `kind` that builds `builds`, as [`Harness::new`] makes a Harness of tests.
    pub fn of_kind(
        kind: CodeKind,
        id: HarnessId,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<Harness, HarnessError> {
        Harness::make(
            id,
            builds,
            HarnessContent::Code {
                kind,
                source,
                cases,
            },
        )
    }

    /// A new Harness at its first revision that builds `builds` and holds `content`, each Case at its first
    /// revision, in the order given. The rules of the content hold, and so do the size limits of a put.
    pub fn make(
        id: HarnessId,
        builds: BTreeSet<InterfaceId>,
        content: HarnessContent,
    ) -> Result<Harness, HarnessError> {
        content.check(id, &builds)?;
        content.check_sizes(id)?;
        let (content, parts) = content.into_parts();
        Ok(Harness {
            id,
            revision: Revision::first(),
            builds,
            content,
            cases: parts
                .into_iter()
                .map(|(id, name, body, cites)| Case {
                    id,
                    revision: Revision::first(),
                    name,
                    cites,
                    body,
                })
                .collect(),
        })
    }

    /// A Harness as a store kept it, revisions included. The rules of [`Harness::make`] hold on it but for the
    /// size limits, which are the rules of a put: a limit lowered later does not make what was kept unreadable.
    pub(crate) fn restore(
        id: HarnessId,
        revision: Revision,
        builds: BTreeSet<InterfaceId>,
        stored: StoredContent,
    ) -> Result<Harness, HarnessError> {
        let (content, revisions) = stored.split();
        content.check(id, &builds)?;
        let (content, parts) = content.into_parts();
        Ok(Harness {
            id,
            revision,
            builds,
            content,
            cases: parts
                .into_iter()
                .zip(revisions)
                .map(|((id, name, body, cites), revision)| Case {
                    id,
                    revision,
                    name,
                    cites,
                    body,
                })
                .collect(),
        })
    }

    /// Its id.
    pub fn id(&self) -> HarnessId {
        self.id
    }

    /// How many times its content has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// What it is for.
    pub fn kind(&self) -> HarnessKind {
        self.content.kind()
    }

    /// The kind of its code; none for a Harness of examples, which has none.
    pub fn code_kind(&self) -> Option<CodeKind> {
        match &self.content {
            Content::Code { kind, .. } => Some(*kind),
            Content::Examples => None,
        }
    }

    /// The Interfaces whose pinned code a sim Harness or a Harness of examples builds into the program it runs, in
    /// the order of their ids. A Harness of tests builds none.
    pub fn builds(&self) -> &BTreeSet<InterfaceId> {
        &self.builds
    }

    /// Its code; none for a Harness of examples.
    pub fn source(&self) -> Option<&Source> {
        self.content.source()
    }

    /// Its Cases, in order.
    pub fn cases(&self) -> &[Case] {
        &self.cases
    }

    /// The Case with this id.
    pub fn case(&self, id: CaseId) -> Option<&Case> {
        self.cases.iter().find(|c| c.id == id)
    }

    /// The ways a sim Harness is not the shape the protocol runs that the Harness alone decides: not exactly one
    /// `sweep` Case, a Case that is neither `sweep` nor `regress-<name>` or whose regress name another has, a Case
    /// that cites no Clause, and no Interface to build. A Harness of tests has none. What needs the Context (one
    /// sim Harness, the Clauses the Cases cite) is asked of [`Context::sim_problems`].
    ///
    /// These are not rules of [`Harness::new`] or of reading a stored Harness: a Harness a store kept before they held is
    /// read as it is, and only a Harness put again is held to them (see `Project::apply`).
    ///
    /// [`Context::sim_problems`]: crate::domain::entity::context::Context::sim_problems
    pub fn sim_problems(&self) -> Vec<SimShapeProblem> {
        if self.kind() != HarnessKind::Sim {
            return Vec::new();
        }
        let harness = self.id;
        let mut out = Vec::new();
        let sweeps: Vec<&Case> = self
            .cases
            .iter()
            .filter(|c| role_of(c.name.as_str()) == Some(CaseRole::Sweep))
            .collect();
        if sweeps.is_empty() {
            out.push(SimShapeProblem::SweepOnce {
                harness,
                count: 0,
                extra: None,
            });
        }
        for extra in sweeps.iter().skip(1) {
            out.push(SimShapeProblem::SweepOnce {
                harness,
                count: sweeps.len(),
                extra: Some(extra.id),
            });
        }
        let mut seen: Vec<RegressName> = Vec::new();
        for c in &self.cases {
            let case = c.id;
            let role = role_of(c.name.as_str());
            match &role {
                None => out.push(SimShapeProblem::CaseName {
                    harness,
                    case,
                    fault: CaseNameFault::NeitherSweepNorRegress,
                }),
                Some(CaseRole::Regress(n)) => {
                    if seen.contains(n) {
                        out.push(SimShapeProblem::CaseName {
                            harness,
                            case,
                            fault: CaseNameFault::RegressNamedTwice,
                        });
                    }
                    seen.push(n.clone());
                }
                Some(CaseRole::Sweep) => {}
            }
            if role.is_some() && c.cites.clauses().is_empty() {
                out.push(SimShapeProblem::CaseCitesNone { harness, case });
            }
        }
        if self.builds.is_empty() {
            out.push(SimShapeProblem::BuildsNone { harness });
        }
        out
    }

    /// Gives the Harness new code and Cases, and keeps the kind of code and the Interfaces it builds; as
    /// [`Harness::put`] does. A Harness of examples is refused ([`HarnessError::KindContent`]): it does not become
    /// code unasked.
    pub fn revise(&mut self, source: Source, cases: Vec<CaseFound>) -> Result<(), HarnessError> {
        let kind = self
            .code_kind()
            .ok_or(HarnessError::KindContent { harness: self.id })?;
        self.revise_as(kind, self.builds.clone(), source, cases)
    }

    /// As [`Harness::revise`], and the Harness becomes code of `kind` that builds `builds`. A change of kind or of
    /// what it builds is a change of the Harness: its revision goes up.
    pub fn revise_as(
        &mut self,
        kind: CodeKind,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), HarnessError> {
        self.put(
            builds,
            HarnessContent::Code {
                kind,
                source,
                cases,
            },
        )
    }

    /// Gives the Harness the content and the Interfaces to build that a put states, whatever it held. A Case whose
    /// id was there keeps its revision when its name, its citations and its body are the same (for a test, the
    /// text of its lines, wherever it now is; for an example, its texts) and goes to the next one when any differs;
    /// a new id starts at the first; an id left out is gone. The Harness goes to its next revision when its kind,
    /// what it builds, its code or its Cases (which, in what order, named what, citing what, holding what) differ,
    /// and stays when nothing does. A change of kind (examples for tests, say) is therefore a revision of the
    /// Harness and of every Case that was there. Content that breaks a rule, or is larger than a put takes,
    /// changes nothing.
    pub fn put(
        &mut self,
        builds: BTreeSet<InterfaceId>,
        content: HarnessContent,
    ) -> Result<(), HarnessError> {
        content.check(self.id, &builds)?;
        content.check_sizes(self.id)?;
        let (content, parts) = content.into_parts();
        let revised: Vec<Case> = parts
            .into_iter()
            .map(|(id, name, body, cites)| {
                let revision = match self.case(id) {
                    None => Revision::first(),
                    Some(old)
                        if old.name == name
                            && old.cites == cites
                            && self.same_body(old, &content, &body) =>
                    {
                        old.revision
                    }
                    Some(old) => old.revision.next(),
                };
                Case {
                    id,
                    revision,
                    name,
                    cites,
                    body,
                }
            })
            .collect();
        let same = content == self.content
            && builds == self.builds
            && revised.len() == self.cases.len()
            && revised.iter().zip(&self.cases).all(|(new, old)| {
                new.id == old.id
                    && new.name == old.name
                    && new.cites == old.cites
                    && same_content(&new.body, &old.body)
            });
        if !same {
            self.revision = self.revision.next();
        }
        self.content = content;
        self.builds = builds;
        self.cases = revised;
        Ok(())
    }

    /// Whether `body`, in a Harness that will hold `content`, is what `old` holds in this one: a test is the text of
    /// its lines, an example its texts.
    fn same_body(&self, old: &Case, content: &Content, body: &CaseBody) -> bool {
        match (&old.body, body) {
            (CaseBody::Lines(then), CaseBody::Lines(now)) => {
                match (self.content.source(), content.source()) {
                    (Some(was), Some(is)) => was.lines(*then) == is.lines(*now),
                    _ => false,
                }
            }
            (CaseBody::Example(then), CaseBody::Example(now)) => then == now,
            (CaseBody::Lines(_), CaseBody::Example(_))
            | (CaseBody::Example(_), CaseBody::Lines(_)) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::vo::id::{ClauseId, ItemId, test_uuid};

    fn cid(n: u128) -> CaseId {
        CaseId::from_uuid(test_uuid(n))
    }

    fn name(s: &str) -> CaseName {
        CaseName::try_from(s).unwrap()
    }

    fn at(first: usize, last: usize) -> LineRange {
        LineRange::new(first, last).unwrap()
    }

    fn src(s: &str) -> Source {
        Source::new(s.to_string())
    }

    /// Cites the Item `item` at revision `rev`, and no Clause.
    fn cites(item: u128, rev: Revision) -> CaseCites {
        CaseCites::new(
            BTreeMap::from([(ItemId::from_uuid(test_uuid(item)), rev)]),
            BTreeMap::new(),
        )
        .unwrap()
    }

    const CODE: &str = "fn push_then_pop() { a }\nfn empty_has_no_top() { b }\n";

    fn stack() -> Harness {
        Harness::new(
            HarnessId::from_uuid(test_uuid(100)),
            src(CODE),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap()
    }

    fn revisions(h: &Harness) -> Vec<u64> {
        h.cases().iter().map(|c| c.revision().number()).collect()
    }

    #[test]
    fn a_new_harness_and_its_cases_start_at_the_first_revision() {
        let h = stack();
        assert_eq!(h.revision(), Revision::first());
        assert_eq!(revisions(&h), [1, 1]);
        assert_eq!(h.case(cid(2)).unwrap().name().as_str(), "empty_has_no_top");
        assert!(h.case(cid(9)).is_none());
    }

    #[test]
    fn a_case_may_cite_several_items_and_clauses() {
        let items = BTreeMap::from([
            (ItemId::from_uuid(test_uuid(50)), Revision::first()),
            (ItemId::from_uuid(test_uuid(51)), Revision::first().next()),
        ]);
        let clauses = BTreeMap::from([(ClauseId::from_uuid(test_uuid(60)), Revision::first())]);
        let h = Harness::new(
            HarnessId::from_uuid(test_uuid(100)),
            src(CODE),
            vec![(
                cid(1),
                name("push_then_pop"),
                at(1, 1),
                CaseCites::new(items, clauses).unwrap(),
            )],
        )
        .unwrap();
        let c = h.case(cid(1)).unwrap().cites();
        assert_eq!((c.items().len(), c.clauses().len()), (2, 1));
    }

    #[test]
    fn a_duplicate_id_or_a_case_past_the_last_line_is_refused() {
        let id = HarnessId::from_uuid(test_uuid(100));
        assert_eq!(
            Harness::new(
                id,
                src(CODE),
                vec![
                    (cid(1), name("a"), at(1, 1), cites(50, Revision::first())),
                    (cid(1), name("b"), at(2, 2), cites(50, Revision::first())),
                ]
            ),
            Err(HarnessError::DuplicateId(cid(1)))
        );
        assert_eq!(
            Harness::new(
                id,
                src(CODE),
                vec![(cid(1), name("a"), at(2, 3), cites(50, Revision::first()))]
            ),
            Err(HarnessError::RangeOutside {
                id: cid(1),
                first: 2,
                last: 3,
                total: 2
            })
        );
    }

    #[test]
    fn the_same_code_and_cases_again_change_no_revision() {
        let mut h = stack();
        let before = h.clone();
        h.revise(
            src(CODE),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap();
        assert_eq!(h, before);
    }

    #[test]
    fn a_moved_case_keeps_its_revision_and_the_harness_goes_to_the_next() {
        let mut h = stack();
        h.revise(
            src(&format!("// tests\n{CODE}")),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(2, 2),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(3, 3),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap();
        assert_eq!(h.revision().number(), 2);
        assert_eq!(revisions(&h), [1, 1]);
    }

    #[test]
    fn a_changed_body_or_citation_goes_to_the_next_revision_and_the_others_stay() {
        let mut h = stack();
        h.revise(
            src("fn push_then_pop() { a2 }\nfn empty_has_no_top() { b }\n"),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (2, vec![2, 1]));
        h.revise(
            src("fn push_then_pop() { a2 }\nfn empty_has_no_top() { b }\n"),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first().next()),
                ),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (3, vec![2, 2]));
    }

    #[test]
    fn a_harness_is_of_tests_unless_made_a_sim_and_a_change_of_kind_is_a_revision() {
        let mut h = stack();
        assert_eq!(h.kind(), HarnessKind::Tests);
        let cases = |h: &Harness| -> Vec<CaseFound> {
            h.cases()
                .iter()
                .map(|c| {
                    (
                        c.id(),
                        c.name().clone(),
                        c.lines().unwrap(),
                        c.cites().clone(),
                    )
                })
                .collect()
        };
        // revising without a kind keeps the kind and, with nothing else changed, the revision
        h.revise(src(CODE), cases(&h)).unwrap();
        assert_eq!((h.kind(), h.revision().number()), (HarnessKind::Tests, 1));
        // a Case of tests cites an Item, so it is not a Case of a sim: the Cases are given again for the sim
        let clauses = |h: &Harness| -> Vec<CaseFound> {
            cases(h)
                .into_iter()
                .map(|(id, name, lines, _)| (id, name, lines, sim_cites()))
                .collect()
        };
        h.revise_as(CodeKind::Sim, BTreeSet::new(), src(CODE), clauses(&h))
            .unwrap();
        assert_eq!((h.kind(), h.revision().number()), (HarnessKind::Sim, 2));
        assert_eq!(revisions(&h), [2, 2]);
        // and the kind stays through a revise that names none
        h.revise(src(CODE), cases(&h)).unwrap();
        assert_eq!((h.kind(), h.revision().number()), (HarnessKind::Sim, 2));
        // a new Harness can be made a sim
        let sim = Harness::of_kind(
            CodeKind::Sim,
            HarnessId::from_uuid(test_uuid(101)),
            BTreeSet::new(),
            src(CODE),
            cases(&h),
        )
        .unwrap();
        assert_eq!(sim.kind(), HarnessKind::Sim);
    }

    /// Citations of a Case of a sim: one Clause, no Item.
    fn sim_cites() -> CaseCites {
        CaseCites::of(
            BTreeMap::new(),
            BTreeMap::from([(ClauseId::from_uuid(test_uuid(60)), Revision::first())]),
        )
    }

    fn iid(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    fn sim_case(n: u128) -> CaseFound {
        (cid(n), name("sweep"), at(1, 1), sim_cites())
    }

    #[test]
    fn a_case_of_tests_cites_an_item_and_a_case_of_a_sim_none() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let bare = (
            cid(1),
            name("a"),
            at(1, 1),
            CaseCites::of(BTreeMap::new(), BTreeMap::new()),
        );
        assert_eq!(
            Harness::new(id, src(CODE), vec![bare]),
            Err(HarnessError::NoItem(cid(1)))
        );
        let with_item = (cid(2), name("a"), at(1, 1), cites(50, Revision::first()));
        assert_eq!(
            Harness::of_kind(
                CodeKind::Sim,
                id,
                BTreeSet::new(),
                src(CODE),
                vec![with_item]
            ),
            Err(HarnessError::SimCitesItem(cid(2)))
        );
        let sim = Harness::of_kind(
            CodeKind::Sim,
            id,
            BTreeSet::new(),
            src(CODE),
            vec![sim_case(3)],
        )
        .unwrap();
        assert!(sim.case(cid(3)).unwrap().cites().items().is_empty());
    }

    #[test]
    fn only_a_sim_builds_and_what_it_builds_is_part_of_its_content() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let one = BTreeSet::from([iid(7)]);
        assert_eq!(
            Harness::of_kind(
                CodeKind::Tests,
                id,
                one.clone(),
                src(CODE),
                vec![(cid(1), name("a"), at(1, 1), cites(50, Revision::first()))]
            ),
            Err(HarnessError::BuildsOfTests(id))
        );
        let mut sim =
            Harness::of_kind(CodeKind::Sim, id, one.clone(), src(CODE), vec![sim_case(3)]).unwrap();
        assert_eq!(sim.builds(), &one);
        // the same content again changes nothing; another set of Interfaces is a new revision of the Harness
        // and of no Case
        sim.revise_as(CodeKind::Sim, one.clone(), src(CODE), vec![sim_case(3)])
            .unwrap();
        assert_eq!(sim.revision().number(), 1);
        let two = BTreeSet::from([iid(7), iid(8)]);
        sim.revise_as(CodeKind::Sim, two.clone(), src(CODE), vec![sim_case(3)])
            .unwrap();
        assert_eq!((sim.revision().number(), sim.builds()), (2, &two));
        assert_eq!(revisions(&sim), [1]);
        // a plain revise keeps what it builds
        sim.revise(src(CODE), vec![sim_case(3)]).unwrap();
        assert_eq!((sim.revision().number(), sim.builds()), (2, &two));
        // and turning it into tests with builds left is refused, changing nothing
        let before = sim.clone();
        assert_eq!(
            sim.revise_as(
                CodeKind::Tests,
                two,
                src(CODE),
                vec![(cid(3), name("a"), at(1, 1), cites(50, Revision::first()))]
            ),
            Err(HarnessError::BuildsOfTests(id))
        );
        assert_eq!(sim, before);
    }

    /// A sim Harness that builds Interface 7, with these Cases.
    fn sim_of(cases: Vec<CaseFound>) -> Harness {
        Harness::of_kind(
            CodeKind::Sim,
            HarnessId::from_uuid(test_uuid(100)),
            BTreeSet::from([iid(7)]),
            src(CODE),
            cases,
        )
        .unwrap()
    }

    fn case_named(n: u128, text: &str, clauses: bool) -> CaseFound {
        let cites = if clauses {
            sim_cites()
        } else {
            CaseCites::of(BTreeMap::new(), BTreeMap::new())
        };
        (cid(n), name(text), at(1, 1), cites)
    }

    #[test]
    fn a_sweep_and_regress_cases_that_cite_clauses_and_a_build_are_no_problem() {
        let h = sim_of(vec![
            case_named(1, "sweep", true),
            case_named(2, "regress-lease-lost", true),
        ]);
        assert!(h.sim_problems().is_empty());
    }

    #[test]
    fn a_harness_of_tests_has_no_sim_problem_whatever_its_cases_are_called() {
        let h = stack();
        assert_eq!(h.kind(), HarnessKind::Tests);
        assert!(h.sim_problems().is_empty());
    }

    #[test]
    fn a_sim_without_a_sweep_or_with_two_is_a_sweep_once_problem() {
        let id = HarnessId::from_uuid(test_uuid(100));
        assert_eq!(
            sim_of(vec![case_named(1, "regress-a", true)]).sim_problems(),
            [SimShapeProblem::SweepOnce {
                harness: id,
                count: 0,
                extra: None
            }]
        );
        assert_eq!(
            sim_of(vec![
                case_named(1, "sweep", true),
                case_named(2, "sweep", true)
            ])
            .sim_problems(),
            [SimShapeProblem::SweepOnce {
                harness: id,
                count: 2,
                extra: Some(cid(2))
            }]
        );
    }

    #[test]
    fn a_case_name_that_is_no_role_or_repeats_a_regress_name_is_a_case_name_problem() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let problems = sim_of(vec![
            case_named(1, "sweep", true),
            case_named(2, "Regress-A", true),
            case_named(3, "regress-a", true),
            case_named(4, "regress-a", true),
        ])
        .sim_problems();
        assert_eq!(
            problems,
            [
                SimShapeProblem::CaseName {
                    harness: id,
                    case: cid(2),
                    fault: CaseNameFault::NeitherSweepNorRegress
                },
                SimShapeProblem::CaseName {
                    harness: id,
                    case: cid(4),
                    fault: CaseNameFault::RegressNamedTwice
                },
            ]
        );
    }

    #[test]
    fn a_sim_case_that_cites_no_clause_is_a_problem_only_when_it_has_a_role() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let problems = sim_of(vec![
            case_named(1, "sweep", true),
            case_named(2, "regress-b", false),
            case_named(3, "stray", false),
        ])
        .sim_problems();
        assert_eq!(
            problems,
            [
                SimShapeProblem::CaseCitesNone {
                    harness: id,
                    case: cid(2)
                },
                SimShapeProblem::CaseName {
                    harness: id,
                    case: cid(3),
                    fault: CaseNameFault::NeitherSweepNorRegress
                },
            ]
        );
    }

    #[test]
    fn a_sim_that_builds_nothing_is_a_builds_none_problem() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let h = Harness::of_kind(
            CodeKind::Sim,
            id,
            BTreeSet::new(),
            src(CODE),
            vec![case_named(1, "sweep", true)],
        )
        .unwrap();
        assert_eq!(
            h.sim_problems(),
            [SimShapeProblem::BuildsNone { harness: id }]
        );
    }

    #[test]
    fn a_harness_kept_in_a_shape_the_rules_refuse_now_is_still_read() {
        // `restore` holds a Harness to the rules of its kind, not to the shape of a sim: a store from before the
        // shape was refused on a put is read, and `Project::apply` holds only what is put again to it.
        let id = HarnessId::from_uuid(test_uuid(100));
        let h = Harness::restore(
            id,
            Revision::first(),
            BTreeSet::new(),
            StoredContent::Code {
                kind: CodeKind::Sim,
                source: src(CODE),
                cases: vec![(
                    cid(1),
                    Revision::first(),
                    name("stray"),
                    at(1, 1),
                    sim_cites(),
                )],
            },
        )
        .unwrap();
        assert_eq!(h.sim_problems().len(), 3);
    }

    #[test]
    fn content_that_breaks_a_rule_changes_nothing() {
        let mut h = stack();
        let before = h.clone();
        assert!(
            h.revise(
                src("x\n"),
                vec![(cid(1), name("a"), at(1, 2), cites(50, Revision::first()))]
            )
            .is_err()
        );
        assert_eq!(h, before);
    }

    fn hid() -> HarnessId {
        HarnessId::from_uuid(test_uuid(100))
    }

    fn ex_with(input: &str, expected: &str) -> Example {
        Example::new(String::new(), input.to_string(), expected.to_string(), None).unwrap()
    }

    fn ex(expected: &str) -> Example {
        ex_with("", expected)
    }

    /// An example Case that cites one Clause and no Item.
    fn example_case(n: u128, text: &str, expected: &str) -> ExampleFound {
        (cid(n), name(text), ex(expected), sim_cites())
    }

    fn made(cases: Vec<ExampleFound>) -> Result<Harness, HarnessError> {
        Harness::make(
            hid(),
            BTreeSet::from([iid(7)]),
            HarnessContent::Examples(cases),
        )
    }

    /// A Harness of two examples.
    fn tally() -> Harness {
        made(vec![
            example_case(1, "a1", "1\n#exit 0\n"),
            example_case(2, "a2", "2\n#exit 0\n"),
        ])
        .unwrap()
    }

    fn put_examples(h: &mut Harness, cases: Vec<ExampleFound>) -> Result<(), HarnessError> {
        h.put(BTreeSet::from([iid(7)]), HarnessContent::Examples(cases))
    }

    #[test]
    fn a_harness_of_examples_is_made_with_no_code_and_a_case_for_each_example() {
        let h = tally();
        assert_eq!(h.kind(), HarnessKind::Examples);
        assert_eq!(h.code_kind(), None);
        assert!(h.source().is_none());
        assert_eq!(h.builds(), &BTreeSet::from([iid(7)]));
        assert_eq!((h.revision().number(), revisions(&h)), (1, vec![1, 1]));
        let c = h.case(cid(2)).unwrap();
        assert_eq!(c.name().as_str(), "a2");
        assert_eq!(c.lines(), None);
        assert_eq!(c.example(), Some(&ex("2\n#exit 0\n")));
        assert!(matches!(c.body(), CaseBody::Example(_)));
        // and a Case of code has lines and no example
        let tests = stack();
        let t = tests.case(cid(1)).unwrap();
        assert_eq!(t.lines(), Some(at(1, 1)));
        assert_eq!(t.example(), None);
        assert!(matches!(t.body(), CaseBody::Lines(_)));
        assert!(tests.source().is_some());
        assert_eq!(tests.code_kind(), Some(CodeKind::Tests));
    }

    #[test]
    fn a_harness_of_examples_has_no_sim_problem() {
        assert!(tally().sim_problems().is_empty());
    }

    #[test]
    fn an_example_case_that_cites_an_item_or_no_clause_is_refused() {
        let with_item = (
            cid(1),
            name("a1"),
            ex("#exit 0"),
            cites(50, Revision::first()),
        );
        assert_eq!(
            made(vec![with_item]),
            Err(HarnessError::ExampleCitesItem(cid(1)))
        );
        let bare = (
            cid(1),
            name("a1"),
            ex("#exit 0"),
            CaseCites::of(BTreeMap::new(), BTreeMap::new()),
        );
        assert_eq!(
            made(vec![bare]),
            Err(HarnessError::ExampleCitesNone(cid(1)))
        );
    }

    #[test]
    fn the_name_of_an_example_is_one_an_example_has_and_is_there_once() {
        for bad in ["a/b", "a\nb", "a\0b"] {
            assert_eq!(
                made(vec![example_case(1, bad, "#exit 0")]),
                Err(HarnessError::ExampleName {
                    case: cid(1),
                    fault: ExampleNameFault::NotAName
                }),
                "{bad:?}"
            );
        }
        assert_eq!(
            made(vec![
                example_case(1, "a1", "#exit 0"),
                example_case(2, "a1", "#exit 1"),
            ]),
            Err(HarnessError::ExampleName {
                case: cid(2),
                fault: ExampleNameFault::NamedTwice
            })
        );
    }

    #[test]
    fn a_harness_of_examples_builds_an_interface_and_its_cases_have_one_id_each() {
        assert_eq!(
            Harness::make(
                hid(),
                BTreeSet::new(),
                HarnessContent::Examples(vec![example_case(1, "a1", "#exit 0")])
            ),
            Err(HarnessError::ExamplesBuildNone(hid()))
        );
        assert_eq!(
            made(vec![
                example_case(1, "a1", "#exit 0"),
                example_case(1, "a2", "#exit 0"),
            ]),
            Err(HarnessError::DuplicateId(cid(1)))
        );
        // no Case at all is a Harness of no examples, which the rules do not refuse
        assert!(made(vec![]).unwrap().cases().is_empty());
    }

    #[test]
    fn the_same_examples_again_change_no_revision() {
        let mut h = tally();
        let before = h.clone();
        put_examples(
            &mut h,
            vec![
                example_case(1, "a1", "1\n#exit 0\n"),
                example_case(2, "a2", "2\n#exit 0\n"),
            ],
        )
        .unwrap();
        assert_eq!(h, before);
    }

    #[test]
    fn one_changed_text_moves_that_case_and_the_harness_and_no_other_case() {
        let mut h = tally();
        put_examples(
            &mut h,
            vec![
                example_case(1, "a1", "1\n#exit 0\n"),
                example_case(2, "a2", "2\n#exit 1\n"),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (2, vec![1, 2]));
        // the input is as much its content as the expected output
        put_examples(
            &mut h,
            vec![
                (
                    cid(1),
                    name("a1"),
                    ex_with("x", "1\n#exit 0\n"),
                    sim_cites(),
                ),
                example_case(2, "a2", "2\n#exit 1\n"),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (3, vec![2, 2]));
        // so are the errors an example states
        let errors = Example::new(
            String::new(),
            String::new(),
            "2\n#exit 1\n".to_string(),
            Some("e\n".to_string()),
        )
        .unwrap();
        put_examples(
            &mut h,
            vec![
                (
                    cid(1),
                    name("a1"),
                    ex_with("x", "1\n#exit 0\n"),
                    sim_cites(),
                ),
                (cid(2), name("a2"), errors, sim_cites()),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (4, vec![2, 3]));
    }

    #[test]
    fn a_renamed_or_recited_example_and_a_reorder_move_the_harness() {
        let mut h = tally();
        put_examples(
            &mut h,
            vec![
                example_case(2, "a2", "2\n#exit 0\n"),
                example_case(1, "a1", "1\n#exit 0\n"),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (2, vec![1, 1]));
        put_examples(
            &mut h,
            vec![
                example_case(2, "b2", "2\n#exit 0\n"),
                example_case(1, "a1", "1\n#exit 0\n"),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (3, vec![2, 1]));
    }

    #[test]
    fn what_a_harness_of_examples_builds_is_part_of_its_content() {
        let mut h = tally();
        let two = BTreeSet::from([iid(7), iid(8)]);
        h.put(
            two.clone(),
            HarnessContent::Examples(vec![
                example_case(1, "a1", "1\n#exit 0\n"),
                example_case(2, "a2", "2\n#exit 0\n"),
            ]),
        )
        .unwrap();
        assert_eq!((h.revision().number(), h.builds()), (2, &two));
        assert_eq!(revisions(&h), [1, 1]);
    }

    #[test]
    fn a_change_of_kind_between_examples_and_tests_is_a_revision_of_the_harness_and_its_cases() {
        let mut h = tally();
        // examples become a sim; the Case ids stay, and the bodies are others
        h.revise_as(
            CodeKind::Sim,
            BTreeSet::from([iid(7)]),
            src(CODE),
            vec![(cid(1), name("sweep"), at(1, 1), sim_cites())],
        )
        .unwrap();
        assert_eq!(h.kind(), HarnessKind::Sim);
        assert!(h.source().is_some());
        assert_eq!((h.revision().number(), revisions(&h)), (2, vec![2]));
        assert!(h.case(cid(2)).is_none());
        // and back
        put_examples(&mut h, vec![example_case(1, "a1", "1\n#exit 0\n")]).unwrap();
        assert_eq!(h.kind(), HarnessKind::Examples);
        assert!(h.source().is_none());
        assert_eq!((h.revision().number(), revisions(&h)), (3, vec![3]));
        // tests too: a Case of tests that becomes an example is a new revision
        let mut t = stack();
        t.put(
            BTreeSet::from([iid(7)]),
            HarnessContent::Examples(vec![example_case(1, "a1", "#exit 0")]),
        )
        .unwrap();
        assert_eq!(
            (t.kind(), t.revision().number(), revisions(&t)),
            (HarnessKind::Examples, 2, vec![2])
        );
    }

    #[test]
    fn code_put_to_a_harness_of_examples_by_revise_is_refused_and_changes_nothing() {
        let mut h = tally();
        let before = h.clone();
        assert_eq!(
            h.revise(src(CODE), vec![sim_case(1)]),
            Err(HarnessError::KindContent { harness: hid() })
        );
        assert_eq!(h, before);
    }

    #[test]
    fn examples_that_break_a_rule_change_nothing() {
        let mut h = tally();
        let before = h.clone();
        assert!(put_examples(&mut h, vec![example_case(1, "a/b", "#exit 0")]).is_err());
        assert_eq!(h, before);
    }

    /// An example whose input is `bytes` long.
    fn big(n: u128, bytes: usize) -> ExampleFound {
        (
            cid(n),
            name(&format!("big{n}")),
            ex_with(&"x".repeat(bytes), "#exit 0\n"),
            sim_cites(),
        )
    }

    #[test]
    fn a_put_takes_a_text_up_to_the_limit_and_refuses_one_byte_more() {
        assert!(made(vec![big(1, TEXT_LIMIT)]).is_ok());
        assert_eq!(
            made(vec![big(1, TEXT_LIMIT + 1)]),
            Err(HarnessError::ExampleTooLarge {
                case: cid(1),
                part: ExamplePart::Input,
                size: TEXT_LIMIT + 1,
                limit: TEXT_LIMIT
            })
        );
        // a put to a Harness that is is held to it too, and changes nothing when refused
        let mut h = tally();
        let before = h.clone();
        assert!(matches!(
            put_examples(&mut h, vec![big(1, TEXT_LIMIT + 1)]),
            Err(HarnessError::ExampleTooLarge { .. })
        ));
        assert_eq!(h, before);
    }

    #[test]
    fn a_put_takes_the_texts_of_a_harness_up_to_the_limit_together() {
        let each = TEXT_LIMIT - 16;
        let n = HARNESS_LIMIT / each;
        let within: Vec<ExampleFound> = (1..=n as u128).map(|k| big(k, each)).collect();
        assert!(made(within.clone()).is_ok());
        let mut over = within;
        over.push(big(100, 64));
        let Err(HarnessError::ExamplesTooLarge {
            harness,
            size,
            limit,
        }) = made(over)
        else {
            panic!("the examples are past the limit of a harness");
        };
        assert_eq!((harness, limit), (hid(), HARNESS_LIMIT));
        assert!(size > HARNESS_LIMIT);
    }

    #[test]
    fn the_size_limits_are_rules_of_a_put_and_not_of_a_kept_harness() {
        let each = TEXT_LIMIT;
        let cases: Vec<(CaseId, Revision, CaseName, Example, CaseCites)> = (1..=5u128)
            .map(|k| {
                (
                    cid(k),
                    Revision::first(),
                    name(&format!("big{k}")),
                    ex_with(&"x".repeat(each), "#exit 0\n"),
                    sim_cites(),
                )
            })
            .collect();
        // five texts of the most one text may have are past the limit of a Harness, as a put sees it
        assert!(matches!(
            made(
                cases
                    .iter()
                    .map(|(i, _, n, e, c)| (*i, n.clone(), e.clone(), c.clone()))
                    .collect()
            ),
            Err(HarnessError::ExamplesTooLarge { .. })
        ));
        // and a store that kept them is read
        let h = Harness::restore(
            hid(),
            Revision::first(),
            BTreeSet::from([iid(7)]),
            StoredContent::Examples(cases),
        )
        .unwrap();
        assert_eq!(h.cases().len(), 5);
        // one text of a kept Harness past the limit is read as well
        let one = vec![(
            cid(1),
            Revision::first(),
            name("big"),
            ex_with(&"x".repeat(TEXT_LIMIT + 1), "#exit 0\n"),
            sim_cites(),
        )];
        assert!(
            Harness::restore(
                hid(),
                Revision::first(),
                BTreeSet::from([iid(7)]),
                StoredContent::Examples(one)
            )
            .is_ok()
        );
    }

    #[test]
    fn a_kept_harness_of_examples_is_read_with_its_revisions_and_held_to_the_rules() {
        let stored = |cases: Vec<(CaseId, Revision, CaseName, Example, CaseCites)>,
                      builds: BTreeSet<InterfaceId>| {
            Harness::restore(
                hid(),
                Revision::first().next(),
                builds,
                StoredContent::Examples(cases),
            )
        };
        let case = |n: u128, text: &str, cites: CaseCites| {
            (
                cid(n),
                Revision::first().next(),
                name(text),
                ex("#exit 0"),
                cites,
            )
        };
        let h = stored(vec![case(1, "a1", sim_cites())], BTreeSet::from([iid(7)])).unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (2, vec![2]));
        assert_eq!(h.kind(), HarnessKind::Examples);
        let bare = CaseCites::of(BTreeMap::new(), BTreeMap::new());
        assert_eq!(
            stored(vec![case(1, "a1", bare)], BTreeSet::from([iid(7)])),
            Err(HarnessError::ExampleCitesNone(cid(1)))
        );
        assert_eq!(
            stored(vec![case(1, "a/1", sim_cites())], BTreeSet::from([iid(7)])),
            Err(HarnessError::ExampleName {
                case: cid(1),
                fault: ExampleNameFault::NotAName
            })
        );
        assert_eq!(
            stored(vec![case(1, "a1", sim_cites())], BTreeSet::new()),
            Err(HarnessError::ExamplesBuildNone(hid()))
        );
    }
}
