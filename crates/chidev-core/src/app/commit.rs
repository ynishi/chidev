//! The one way the model, and anything else the store keeps, is changed: every use case that changes state is a
//! [`Trigger`], enters through the entry of its kind, and is kept only as a [`Gated`].
//!
//! A use case does not decide which checks apply to it; its trigger does, by its type. An edit is held to
//! [`Moment::Edit`] and told only what it introduced, a confirmation to [`Moment::Pin`] or [`Moment::Resolve`] and
//! judged whole, an explicit question to [`Moment::Check`] and never refused, and a record to no check at all. One
//! entry for each of them runs what its moment asks for and builds the `Gated`, and the store keeps nothing else
//! ([`Store::commit`]). Only this module can build a `Gated`, so a use case cannot keep a change without passing
//! through the entry that asks for its checks, and one that is added without an entry cannot keep anything.
//!
//! What was checked is what is kept. An edit or a confirmation hands over a `ModelDelta`, the part of the model it
//! changes. The entry reads the model as the store has it, lays the delta on it to make the state the checks run
//! on, and keeps the delta and nothing else; the store refuses to keep it unless it still stands where the use case
//! read, so the state kept is the state checked.

use std::collections::BTreeMap;

use super::check::Run;
use super::external::ExternalChecked;
use super::ports::external::{ExternalExecutor, ExternalReport};
use super::ports::{
    AssetKind, AssetName, Changes, IdSource, Invocation, JudgedReport, PinCode, Store,
    StoreRevision,
};
use super::{App, AppError, Found, view};
use crate::domain::check::{Change, CheckView, Finding, Moment, introduced};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::StoreError;
use crate::domain::vo::id::{ChangeRequestId, ContextId, CruxId, InterfaceId};

/// Declares one of the four kinds of trigger: the enum, the word of each variant, and the list of all of them, from
/// one list of variants, so that a variant cannot be left out of the list or have no word.
macro_rules! triggers {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $word:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $name {
            /// Every one, in the order they are declared.
            pub const ALL: &'static [$name] = &[ $( $name::$variant, )+ ];

            /// The word it is stored by. Once written it is not changed: see [`Trigger::word`].
            pub fn word(self) -> &'static str {
                match self {
                    $( $name::$variant => $word, )+
                }
            }
        }
    };
}

triggers! {
    /// A use case that edits the model: it changes what the model says, and is held to [`Moment::Edit`]. It reports,
    /// and a mandatory check or an external gate refuses it, only for what it introduced.
    EditTrigger {
        /// [`App::add_context`](super::App::add_context).
        AddContext = "add_context",
        /// [`App::set_uses`](super::App::set_uses).
        SetUses = "set_uses",
        /// [`App::base_on_contract`](super::App::base_on_contract).
        BaseOnContract = "base_on_contract",
        /// [`App::put_contract`](super::App::put_contract).
        PutContract = "put_contract",
        /// [`App::put_interface`](super::App::put_interface) and [`App::put_interface_of`](super::App::put_interface_of).
        PutInterface = "put_interface",
        /// [`App::put_harness`](super::App::put_harness) and [`App::put_harness_of`](super::App::put_harness_of).
        PutHarness = "put_harness",
        /// [`App::apply_change_set`](super::App::apply_change_set).
        ApplyChangeSet = "apply_change_set",
        /// [`App::retire_context`](super::App::retire_context).
        RetireContext = "retire_context",
        /// [`App::retire_contract`](super::App::retire_contract).
        RetireContract = "retire_contract",
        /// [`App::retire_interface`](super::App::retire_interface).
        RetireInterface = "retire_interface",
        /// [`App::retire_harness`](super::App::retire_harness).
        RetireHarness = "retire_harness",
        /// [`App::items_rekey`](super::App::items_rekey).
        RekeyItems = "rekey_items",
        /// [`App::put_policy`](super::App::put_policy).
        PutPolicy = "put_policy",
        /// [`App::open_change_request`](super::App::open_change_request). The record of a change request is edited
        /// like the model is: it is judged by the checks of an edit, and an external gate sees it.
        OpenChangeRequest = "open_change_request",
        /// The decision of a Crux, as an earlier version recorded it, when the Crux was a part of a change request
        /// and the use case was `decide_change_request`. No use case runs under it any more
        /// ([`EditTrigger::DecideCrux`] took its place), but the word is in the database beside the checks of those
        /// runs, and a word once written is never changed: it is read back as it was.
        DecideChangeRequest = "decide_change_request",
        /// [`App::raise_crux`](super::App::raise_crux): a Crux is raised in a change request.
        RaiseCrux = "raise_crux",
        /// [`App::decide_crux`](super::App::decide_crux): a Crux is decided, and the change request it was raised in
        /// takes what the option chosen asks.
        DecideCrux = "decide_crux",
        /// [`App::assume`](super::App::assume).
        Assume = "assume",
        /// [`App::reopen_change_request`](super::App::reopen_change_request).
        ReopenChangeRequest = "reopen_change_request",
    }
}

triggers! {
    /// A use case that confirms the model as it stands: it is held to the moment of what it confirms and judged
    /// whole, so that what is wrong with the model is said where something is claimed of it.
    ConfirmTrigger {
        /// [`App::pin`](super::App::pin); held to [`Moment::Pin`].
        Pin = "pin",
        /// [`App::resolve_change_request`](super::App::resolve_change_request); held to [`Moment::Resolve`].
        ResolveChangeRequest = "resolve_change_request",
    }
}

triggers! {
    /// A use case that asks for every check on the model as it stands, and keeps what they said: held to
    /// [`Moment::Check`], and never refused, because it changes no asset.
    AskTrigger {
        /// [`App::check_run`](super::App::check_run).
        CheckRun = "check_run",
    }
}

triggers! {
    /// A use case that keeps a fact, or makes the Project, and runs no check: what it keeps is not an edit of the
    /// model, or there is no model before it to compare with. A record changes no asset of the model (see
    /// [`RecordChanges`]); only [`RecordTrigger::Init`] makes the Project, through its own entry.
    RecordTrigger {
        /// [`App::init`](super::App::init): there is no Project before it, and an empty one introduces nothing.
        Init = "init",
        /// [`App::measure`](super::App::measure).
        Measure = "measure",
        /// [`App::simulate`](super::App::simulate).
        Simulate = "simulate",
        /// [`App::finding_report`](super::App::finding_report): what an agent judged, kept as the run of a rule.
        ReportFinding = "report_finding",
    }
}

impl ConfirmTrigger {
    /// The moment it is held to.
    pub fn moment(self) -> Moment {
        match self {
            ConfirmTrigger::Pin => Moment::Pin,
            ConfirmTrigger::ResolveChangeRequest => Moment::Resolve,
        }
    }
}

/// A public use case that changes what the store keeps. It carries no data: it is a name, stored with the checks
/// the use case ran, so that a record says which use case it came from.
///
/// The kind of a trigger is the kind of check it is held to, and the moment follows from the type
/// ([`Trigger::moment`]); no table pairs them. The variants of each kind are its use cases, listed where the kind is
/// defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Trigger {
    /// A use case that edits the model.
    Edit(EditTrigger),
    /// A use case that confirms the model.
    Confirm(ConfirmTrigger),
    /// A use case that asks for the checks.
    Ask(AskTrigger),
    /// A use case that records a fact.
    Record(RecordTrigger),
}

impl Trigger {
    /// The moment its checks run at; none for a record, which runs none.
    pub fn moment(&self) -> Option<Moment> {
        match self {
            Trigger::Edit(_) => Some(Moment::Edit),
            Trigger::Confirm(c) => Some(c.moment()),
            Trigger::Ask(_) => Some(Moment::Check),
            Trigger::Record(_) => None,
        }
    }

    /// The word it is stored by. **A word is never changed once written**: it is in the database beside every check
    /// a use case ran, and [`Trigger::from_word`] has to read what an older version wrote. To rename a trigger,
    /// give it the new word and keep the old one readable in `from_word`.
    pub fn word(&self) -> &'static str {
        match self {
            Trigger::Edit(t) => t.word(),
            Trigger::Confirm(t) => t.word(),
            Trigger::Ask(t) => t.word(),
            Trigger::Record(t) => t.word(),
        }
    }

    /// The trigger a word names; none for any other.
    pub fn from_word(word: &str) -> Option<Trigger> {
        Trigger::each().find(|t| t.word() == word)
    }

    /// Every trigger: those of each kind, as the kind lists them.
    pub fn all() -> Vec<Trigger> {
        Trigger::each().collect()
    }

    /// Every trigger, without collecting them: [`Trigger::from_word`] reads each stored row through it.
    fn each() -> impl Iterator<Item = Trigger> {
        EditTrigger::ALL
            .iter()
            .copied()
            .map(Trigger::Edit)
            .chain(ConfirmTrigger::ALL.iter().copied().map(Trigger::Confirm))
            .chain(AskTrigger::ALL.iter().copied().map(Trigger::Ask))
            .chain(RecordTrigger::ALL.iter().copied().map(Trigger::Record))
    }
}

/// What a use case has done that the store is to keep, with the proof that the checks its trigger asks for were run.
///
/// Only this module makes one (`Gated::new` and the fields are private), and [`Store::commit`] keeps nothing
/// else: a use case cannot keep a change without going through an entry of [`App`]. The fields are for the store to
/// read.
///
/// ```compile_fail,E0624
/// use chidev_core::app::commit::{Gated, RecordTrigger, Trigger};
/// use chidev_core::app::ports::Changes;
///
/// // `Gated::new` is private to `app::commit`.
/// let _ = Gated::new(Trigger::Record(RecordTrigger::Init), Changes::default());
/// ```
///
/// ```compile_fail,E0451
/// use chidev_core::app::commit::{Gated, RecordTrigger, Trigger};
/// use chidev_core::app::ports::Changes;
///
/// // nor can one be written out: its fields are private.
/// let _ = Gated {
///     trigger: Trigger::Record(RecordTrigger::Init),
///     changes: Changes::default(),
/// };
/// ```
#[derive(Debug)]
pub struct Gated {
    trigger: Trigger,
    changes: Changes,
}

impl Gated {
    /// Settles `changes` as the work of `trigger`: each check it ran is recorded as that trigger's.
    fn new(trigger: Trigger, mut changes: Changes) -> Gated {
        for invocation in &mut changes.invocations {
            invocation.trigger = Some(trigger);
        }
        Gated { trigger, changes }
    }

    /// The use case that made the change.
    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    /// What it changed.
    pub fn changes(&self) -> &Changes {
        &self.changes
    }

    /// What it changed, for the store to keep without a copy.
    pub fn into_changes(self) -> Changes {
        self.changes
    }
}

/// What a record keeps. It has no Project, Pin, change request or name: a record is not an edit of the model, and
/// the type cannot carry one.
///
/// ```compile_fail,E0560
/// use chidev_core::app::commit::RecordChanges;
///
/// // A record has no model to change; only `commit_init` makes the Project.
/// let _ = RecordChanges {
///     project: None,
///     ..RecordChanges::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct RecordChanges {
    /// Measurements made, appended.
    pub measurements: Vec<Measurement>,
    /// Reports of judged rules, each replacing the report of its rule.
    pub judged: Vec<JudgedReport>,
    /// The runs the use case itself made of the checks (a judged rule's report is the run of that rule).
    pub invocations: Vec<Invocation>,
}

/// What names an asset in the store: its Context, its kind and its name. The key of a name in a [`ModelDelta`], and
/// the primary key of the store's table of names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NameKey {
    context: ContextId,
    kind: AssetKind,
    name: String,
}

/// The part of the model a use case changes: the Project (whole), and the Pins, change requests, Cruxes and asset
/// names it makes or replaces.
///
/// **What replaces what is decided here, once.** Each map is keyed by what the store keys its table by (the
/// Interface of a Pin, the id of a change request or of a Crux, the Context, kind and name of an asset's name), and the key is
/// taken from the value when it is put in, so a caller cannot give a value under another's key.
/// [`ModelDelta::apply`] replaces, in the model that was read, the element of the same key; the store writes each
/// element under that key, and keeps whatever else it has. So the state that is checked (`apply`) and the state
/// that is kept (the store's rows) are the same by one rule.
#[derive(Debug, Clone, Default)]
pub(super) struct ModelDelta {
    project: Option<Project>,
    pins: BTreeMap<InterfaceId, (Pin, PinCode)>,
    change_requests: BTreeMap<ChangeRequestId, ChangeRequest>,
    cruxes: BTreeMap<CruxId, Crux>,
    names: BTreeMap<NameKey, AssetName>,
}

impl ModelDelta {
    /// Nothing changed.
    pub(super) fn new() -> ModelDelta {
        ModelDelta::default()
    }

    /// The Project as it is after the use case.
    pub(super) fn with_project(mut self, project: Project) -> ModelDelta {
        self.project = Some(project);
        self
    }

    /// A Pin made, with the code it accepted. It replaces the Pin of its Interface, and so the code.
    pub(super) fn with_pin(mut self, pin: Pin, code: String) -> ModelDelta {
        let code = PinCode {
            pin: pin.id(),
            interface: pin.interface(),
            code,
        };
        self.pins.insert(pin.interface(), (pin, code));
        self
    }

    /// A change request opened or changed. It replaces the change request of its id.
    pub(super) fn with_change_request(mut self, cr: ChangeRequest) -> ModelDelta {
        self.change_requests.insert(cr.id(), cr);
        self
    }

    /// A Crux raised or decided. It replaces the Crux of its id.
    pub(super) fn with_crux(mut self, crux: Crux) -> ModelDelta {
        self.cruxes.insert(crux.id(), crux);
        self
    }

    /// Names given to the assets a use case makes.
    pub(super) fn with_names(mut self, names: impl IntoIterator<Item = AssetName>) -> ModelDelta {
        for name in names {
            let key = NameKey {
                context: name.context,
                kind: name.kind,
                name: name.name.clone(),
            };
            self.names.insert(key, name);
        }
        self
    }

    /// The names the use case gives, which the store does not have yet.
    fn pending(&self) -> Vec<AssetName> {
        self.names.values().cloned().collect()
    }

    /// The model that was read, with this laid on it: the state the checks run on, and the state the store will
    /// hold once the delta is kept.
    fn apply(&self, read: &ModelState) -> ModelState {
        let mut after = read.clone();
        if let Some(project) = &self.project {
            after.project = project.clone();
        }
        after
            .pins
            .retain(|k| !self.pins.contains_key(&k.interface()));
        after
            .pins
            .extend(self.pins.values().map(|(pin, _)| pin.clone()));
        after
            .change_requests
            .retain(|c| !self.change_requests.contains_key(&c.id()));
        after
            .change_requests
            .extend(self.change_requests.values().cloned());
        after.cruxes.retain(|q| !self.cruxes.contains_key(&q.id()));
        after.cruxes.extend(self.cruxes.values().cloned());
        after
    }

    /// What the store is to keep, with the records of the checks that were run.
    fn into_changes(self, invocations: Vec<Invocation>, external: Vec<ExternalReport>) -> Changes {
        let (pins, pin_codes) = self.pins.into_values().unzip();
        Changes {
            project: self.project,
            pins,
            pin_codes,
            change_requests: self.change_requests.into_values().collect(),
            cruxes: self.cruxes.into_values().collect(),
            names: self.names.into_values().collect(),
            invocations,
            external,
            ..Changes::default()
        }
    }
}

/// The model as the store has it, or as it would be: the parts the checks of an edit and a confirmation read.
#[derive(Debug, Clone)]
struct ModelState {
    project: Project,
    pins: Vec<Pin>,
    change_requests: Vec<ChangeRequest>,
    cruxes: Vec<Crux>,
}

impl ModelState {
    fn view(&self) -> CheckView<'_> {
        view(
            &self.project,
            &self.pins,
            &self.change_requests,
            &self.cruxes,
        )
    }
}

/// Where a gate stands when it settles a run, by the kind of entry: an edit is judged on the state after it and held
/// to the state before it (an external check compares with that, see [`App::judged_externals`]); a confirmation is
/// judged on the one state it leaves, at its own moment. Each carries the store revision the use case read at.
#[derive(Clone, Copy)]
enum Gating<'a> {
    /// An edit: its moment is [`Moment::Edit`].
    Edit {
        at: StoreRevision,
        change: &'a Change<'a>,
    },
    /// A confirmation, at the moment of its trigger.
    Confirm {
        moment: Moment,
        at: StoreRevision,
        view: &'a CheckView<'a>,
    },
}

impl<'a> Gating<'a> {
    fn moment(self) -> Moment {
        match self {
            Gating::Edit { .. } => Moment::Edit,
            Gating::Confirm { moment, .. } => moment,
        }
    }

    fn at(self) -> StoreRevision {
        match self {
            Gating::Edit { at, .. } | Gating::Confirm { at, .. } => at,
        }
    }

    /// The state that is judged and kept.
    fn after(self) -> &'a CheckView<'a> {
        match self {
            Gating::Edit { change, .. } => &change.after,
            Gating::Confirm { view, .. } => view,
        }
    }
}

/// What the checks of a moment gave: the findings for the caller, the records to keep with the change.
struct Gate {
    found: Vec<Found>,
    invocations: Vec<Invocation>,
    external: Vec<ExternalReport>,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Keeps an edit of the model. The checks of [`Moment::Edit`] run on the model as the store has it and on the
    /// model with `delta` laid on it, and the edit is told, and a mandatory check or an external gate refuses it, only
    /// for what the edit introduced. Answers with the findings that were reported.
    ///
    /// `at` is the revision the use case began to read at; the delta is kept only if the store still stands there.
    pub(super) async fn commit_edit(
        &mut self,
        trigger: EditTrigger,
        at: StoreRevision,
        delta: ModelDelta,
    ) -> Result<Vec<Found>, AppError> {
        let before = self.read_state(at).await?;
        let after = delta.apply(&before);
        let change = Change {
            before: before.view(),
            after: after.view(),
        };
        let run_before = self.registry.run(Moment::Edit, at, &change.before);
        let run = self.registry.run(Moment::Edit, at, &change.after);
        let reported = introduced(&run_before.findings, &run.findings);
        let gating = Gating::Edit {
            at,
            change: &change,
        };
        let gate = self.gate(&gating, run, reported, &delta.pending()).await?;
        let changes = delta.into_changes(gate.invocations, gate.external);
        self.settle(at, Trigger::Edit(trigger), changes).await?;
        Ok(gate.found)
    }

    /// Keeps a confirmation. The checks of the moment of `trigger` run on the model with `delta` laid on it, and
    /// judge it whole: everything they find is reported, and a mandatory check or an external gate refuses for
    /// any of it.
    ///
    /// `resolving` is the change request the confirmation resolves, for the checks that judge only unresolved
    /// requests to judge it too (the state they read has it resolved already); none for any other confirmation. It
    /// is for the builtin checks: an external check is not told (see [`CheckView::resolving`]).
    pub(super) async fn commit_confirm(
        &mut self,
        trigger: ConfirmTrigger,
        at: StoreRevision,
        delta: ModelDelta,
        resolving: Option<ChangeRequestId>,
    ) -> Result<Vec<Found>, AppError> {
        let moment = trigger.moment();
        let before = self.read_state(at).await?;
        let after = delta.apply(&before);
        let view = CheckView {
            resolving,
            ..after.view()
        };
        let run = self.registry.run(moment, at, &view);
        let reported = run.findings.clone();
        let gating = Gating::Confirm {
            moment,
            at,
            view: &view,
        };
        let gate = self.gate(&gating, run, reported, &delta.pending()).await?;
        let changes = delta.into_changes(gate.invocations, gate.external);
        self.settle(at, Trigger::Confirm(trigger), changes).await?;
        Ok(gate.found)
    }

    /// Runs every check on the model as it stands, the external ones too, and keeps what they said. It changes no
    /// asset, so nothing it finds refuses it: a person asked, and the answer is kept even when a gate found an
    /// error.
    pub(super) async fn commit_ask(
        &mut self,
        trigger: AskTrigger,
        at: StoreRevision,
    ) -> Result<ExternalChecked, AppError> {
        let state = self.read_state(at).await?;
        let measured = self.measured(&state.project).await?;
        let view = CheckView {
            measurements: &measured.measurements,
            measure_settings: &measured.settings,
            sim_settings: &measured.sim_settings,
            ..state.view()
        };
        let (p, crs, cruxes) = (&state.project, &state.change_requests, &state.cruxes);
        let mut run = self.registry.run(Moment::Check, at, &view);
        let external = self.run_external(Moment::Check, at, &view, &[]).await?;
        let mut findings = self.found(&run.findings, p, crs, &[]).await?;
        findings.extend(
            self.external_findings(&external, p, crs, &[], false)
                .await?,
        );
        findings.extend(self.judged_standing(p, crs, cruxes, at).await?.0);
        run.invocations
            .extend(self.external_invocations(&external, Moment::Check, at));
        let changes = Changes {
            invocations: run.invocations,
            external: external.clone(),
            ..Changes::default()
        };
        self.settle(at, Trigger::Ask(trigger), changes).await?;
        Ok(ExternalChecked { findings, external })
    }

    /// Keeps a record. No check is run, and the record cannot carry a change to the model ([`RecordChanges`]).
    pub(super) async fn commit_record(
        &mut self,
        trigger: RecordTrigger,
        at: StoreRevision,
        record: RecordChanges,
    ) -> Result<(), AppError> {
        let RecordChanges {
            measurements,
            judged,
            invocations,
        } = record;
        let changes = Changes {
            measurements,
            judged,
            invocations,
            ..Changes::default()
        };
        self.settle(at, Trigger::Record(trigger), changes).await
    }

    /// Keeps the Project a store begins with. The one use case that makes a Project has this entry of its own, and
    /// runs no check: there is no model before it, and an empty Project introduces nothing.
    pub(super) async fn commit_init(
        &mut self,
        at: StoreRevision,
        project: Project,
    ) -> Result<(), AppError> {
        let changes = Changes {
            project: Some(project),
            ..Changes::default()
        };
        self.settle(at, Trigger::Record(RecordTrigger::Init), changes)
            .await
    }

    /// The model as the store had it at `at`, the revision the use case read at. The parts are read one after the
    /// other, so a write landing in between would leave a mix no store ever held; a store that has moved past `at`
    /// is a conflict here, before any check judges the mix or an external program is run on it, as the commit would
    /// be one later.
    async fn read_state(&self, at: StoreRevision) -> Result<ModelState, AppError> {
        let project = self.project().await?;
        let pins = self.store.pins().await?;
        let (change_requests, cruxes) = self.requests_and_cruxes().await?;
        let state = ModelState {
            project,
            pins,
            change_requests,
            cruxes,
        };
        if self.store.revision().await? != at {
            return Err(AppError::Store(StoreError::Conflict));
        }
        Ok(state)
    }

    /// The one place a [`Gated`] is made, and the one place the store is written. `at` is the revision the use case
    /// read at: a store that has moved since refuses it.
    async fn settle(
        &self,
        at: StoreRevision,
        trigger: Trigger,
        changes: Changes,
    ) -> Result<(), AppError> {
        Ok(self.store.commit(at, Gated::new(trigger, changes)).await?)
    }

    /// What every gate does with a run of the builtin checks: a `reported` finding of a mandatory check at `error`
    /// refuses the use case before anything is kept (and before an external program is run); then the external
    /// checks of the moment run on the state after, and one that is a gate and has something to say refuses it; what
    /// is left is the findings for the caller and the records to keep. The records are those of the whole `run`,
    /// however much of it is `reported`. Which findings refuse is [`Registry::refusing`](super::Registry::refusing)'s
    /// to say.
    async fn gate(
        &self,
        gating: &Gating<'_>,
        mut run: Run,
        reported: Vec<Finding>,
        pending: &[AssetName],
    ) -> Result<Gate, AppError> {
        let (moment, at, after) = (gating.moment(), gating.at(), gating.after());
        let (project, crs) = (after.project, after.change_requests);
        let refusing = self.registry.refusing(&reported);
        if !refusing.is_empty() {
            return Err(AppError::Refused(
                self.found(&refusing, project, crs, pending).await?,
            ));
        }
        let external = self.run_external(moment, at, after, pending).await?;
        let judged = match *gating {
            Gating::Edit { change, .. } => self.judged_externals(&change.before, &external).await?,
            Gating::Confirm { .. } => external.clone(),
        };
        if judged.iter().any(|r| {
            self.registry
                .external()
                .iter()
                .any(|rule| rule.descriptor().id.as_str() == r.rule && rule.gate())
                && !r.findings.is_empty()
        }) {
            let refusing = self
                .external_findings(&judged, project, crs, pending, false)
                .await?;
            return Err(AppError::ExternalRefused {
                findings: refusing,
                external,
            });
        }
        run.invocations
            .extend(self.external_invocations(&external, moment, at));
        let mut found = self.found(&reported, project, crs, pending).await?;
        found.extend(
            self.external_findings(&judged, project, crs, pending, false)
                .await?,
        );
        Ok(Gate {
            external,
            found,
            invocations: run.invocations,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::{PinId, ProjectId};
    use crate::domain::vo::pin_basis::PinBasis;
    use crate::domain::vo::revision::Revision;
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// One trigger of each kind, and the moment it is held to: the type says it, no table pairs them.
    #[test]
    fn a_trigger_is_held_to_the_moment_its_type_gives() {
        assert_eq!(
            Trigger::Edit(EditTrigger::PutContract).moment(),
            Some(Moment::Edit)
        );
        assert_eq!(
            Trigger::Confirm(ConfirmTrigger::Pin).moment(),
            Some(Moment::Pin)
        );
        assert_eq!(
            Trigger::Confirm(ConfirmTrigger::ResolveChangeRequest).moment(),
            Some(Moment::Resolve)
        );
        assert_eq!(
            Trigger::Ask(AskTrigger::CheckRun).moment(),
            Some(Moment::Check)
        );
        assert_eq!(Trigger::Record(RecordTrigger::Init).moment(), None);
    }

    #[test]
    fn every_trigger_is_read_back_from_its_word_and_no_word_is_twice() {
        let all = Trigger::all();
        assert!(!all.is_empty());
        for t in &all {
            assert_eq!(Trigger::from_word(t.word()), Some(*t), "{}", t.word());
        }
        let words: BTreeSet<&str> = all.iter().map(Trigger::word).collect();
        assert_eq!(words.len(), all.len(), "a word names two triggers");
        assert_eq!(Trigger::from_word("put"), None);
    }

    /// A word written by an earlier version stays readable, though no use case runs under it now; the use cases of the
    /// Crux have words of their own.
    #[test]
    fn the_word_of_the_decision_of_a_change_request_is_still_read_and_the_cruxes_have_their_own() {
        assert_eq!(
            Trigger::from_word("decide_change_request"),
            Some(Trigger::Edit(EditTrigger::DecideChangeRequest))
        );
        assert_eq!(
            Trigger::from_word("raise_crux"),
            Some(Trigger::Edit(EditTrigger::RaiseCrux))
        );
        assert_eq!(
            Trigger::from_word("decide_crux"),
            Some(Trigger::Edit(EditTrigger::DecideCrux))
        );
        for t in [EditTrigger::RaiseCrux, EditTrigger::DecideCrux] {
            assert_eq!(Trigger::Edit(t).moment(), Some(Moment::Edit));
        }
    }

    /// The list is the kinds' own lists, so a trigger declared in a kind is in it.
    #[test]
    fn the_list_of_triggers_is_the_lists_of_the_kinds() {
        let all = Trigger::all();
        let total = EditTrigger::ALL.len()
            + ConfirmTrigger::ALL.len()
            + AskTrigger::ALL.len()
            + RecordTrigger::ALL.len();
        assert_eq!(all.len(), total);
        for t in EditTrigger::ALL {
            assert!(all.contains(&Trigger::Edit(*t)));
        }
        for t in RecordTrigger::ALL {
            assert!(all.contains(&Trigger::Record(*t)));
        }
    }

    #[test]
    fn a_gated_change_records_its_checks_under_its_trigger() {
        use crate::app::ports::{Invocation, Outcome};
        use crate::domain::check::Executor;
        use crate::domain::vo::finding::RuleId;

        let invocation = Invocation {
            check: RuleId::from_static("test.rule"),
            moment: Moment::Edit,
            trigger: None,
            read_at: StoreRevision(0),
            notes: 0,
            warnings: 0,
            errors: 0,
            outcome: Outcome::Ok,
            executor: Executor::Builtin,
            judge: None,
            external: None,
            rule_digest: None,
        };
        let changes = Changes {
            invocations: vec![invocation],
            ..Changes::default()
        };
        let trigger = Trigger::Edit(EditTrigger::PutPolicy);
        let gated = Gated::new(trigger, changes);
        assert_eq!(gated.trigger(), trigger);
        assert_eq!(gated.changes().invocations[0].trigger, Some(trigger));
    }

    fn pin(interface: u128, pin: u128) -> Pin {
        Pin::new(
            PinId::from_uuid(id(pin)),
            InterfaceId::from_uuid(id(interface)),
            Digest::of(b"code"),
            PinBasis::new(
                Revision::first(),
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
        )
    }

    /// A value is kept under the key its own content gives: a Pin of an Interface replaces the Pin of that
    /// Interface and no other, and its code goes with it.
    #[test]
    fn a_delta_replaces_the_element_of_the_same_key_and_leaves_the_rest() {
        let project = Project::new(ProjectId::from_uuid(id(1)));
        let read = ModelState {
            project: project.clone(),
            pins: vec![pin(10, 100), pin(11, 101)],
            change_requests: Vec::new(),
            cruxes: Vec::new(),
        };
        let delta = ModelDelta::new().with_pin(pin(10, 200), "new code".to_string());
        let after = delta.apply(&read);
        let ids: BTreeSet<_> = after.pins.iter().map(|p| p.id()).collect();
        assert_eq!(
            ids,
            BTreeSet::from([PinId::from_uuid(id(101)), PinId::from_uuid(id(200))])
        );
        // The part the delta does not name is as it was.
        assert_eq!(after.project, project);
        let changes = delta.into_changes(Vec::new(), Vec::new());
        assert_eq!(changes.pins.len(), 1);
        assert_eq!(
            changes.pin_codes[0].interface,
            InterfaceId::from_uuid(id(10))
        );
        assert_eq!(changes.pin_codes[0].pin, PinId::from_uuid(id(200)));
        assert!(changes.project.is_none());
    }

    #[test]
    fn a_name_given_twice_in_a_delta_is_kept_once_under_its_key() {
        let name = |n: u128| AssetName {
            context: ContextId::from_uuid(id(1)),
            kind: AssetKind::Contract,
            name: "stack".to_string(),
            id: id(n),
        };
        let delta = ModelDelta::new().with_names([name(5), name(6)]);
        assert_eq!(delta.pending().len(), 1);
        assert_eq!(delta.pending()[0].id, id(6));
    }
}
