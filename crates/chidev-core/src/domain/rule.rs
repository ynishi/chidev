//! The rules of the Project that are not checks: the invariants a change must keep (an operation that would break
//! one is refused) and the rules an input must meet before the domain sees it. They are registered like the checks
//! (an id, a line on what they protect, a help on why and how to fix), so that a refusal and a finding are the same
//! thing to a reader: a violation of a rule, found by its id.
//!
//! A check is a [`Descriptor`](crate::domain::check::Descriptor) and lives with the checks; an invariant or an input
//! rule is a [`Rule`] and lives here. The application lists both (`rules`) and explains either (`explain`).
//!
//! An id, once published, keeps its meaning and is never reused. The keys of the values a rule carries
//! ([`Rule::values`]) may be added to and are never removed, so a program that reads a violation by key goes on
//! working (the contract AIP-193 gives `ErrorInfo.metadata`).

use crate::domain::check::Level;
use crate::domain::vo::finding::RuleId;

/// What a rule is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleKind {
    /// What a change must never break: an operation that would break it is refused. Its level is always `error`,
    /// and no policy lowers it.
    Invariant,
    /// What may be kept but should not be left: found by a check, at the level a policy gives.
    Check,
    /// What an input must be before an operation is tried: its shape and the names it uses. Refused with `error`.
    Input,
}

impl RuleKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            RuleKind::Invariant => "invariant",
            RuleKind::Check => "check",
            RuleKind::Input => "input",
        }
    }
}

/// A rule that is not a check, described.
///
/// The `message` is the template of the sentence a refusal says: `{s0}`, `{s1}` ... are the names of the
/// violation's subjects in order, `{subjects}` all of them, `{others}` all but the first, and `{key}` the value of
/// that key (a list of things is joined with commas). The application fills it in with the names it has; the
/// domain only keeps the words.
#[derive(Debug)]
pub struct Rule {
    /// The id; `<family>.<name>`.
    pub id: RuleId,
    /// What kind of rule it is.
    pub kind: RuleKind,
    /// What it protects, in a line.
    pub summary: &'static str,
    /// Why it holds and how to fix a violation.
    pub help: &'static str,
    /// The sentence a violation says, with `{..}` holes.
    pub message: &'static str,
    /// The keys of the values a violation of it carries.
    pub values: &'static [&'static str],
    /// An operation (JSON) that violates it, for `explain`.
    pub example: &'static str,
}

impl Rule {
    /// How serious a violation is: always `error`.
    pub fn level(&self) -> Level {
        Level::Error
    }
}

/// A rule id with no rule of its own yet: a refusal of the domain that no variant maps to a rule. It carries the
/// text of the error under `error`, and is a bug to be mapped, tracked by the test that lists the variants still
/// here.
pub const UNMAPPED: RuleId = RuleId::from_static("domain.unmapped");
/// The same for a refusal of the application that has no rule yet.
pub const APP_UNMAPPED: RuleId = RuleId::from_static("app.unmapped");
/// A change request is resolved and the operation needs it open.
pub const CR_ALREADY_RESOLVED: RuleId = RuleId::from_static("cr.already-resolved");
/// A change request is to be resolved while a crux raised in it has no decision.
pub const CR_CRUX_UNANSWERED: RuleId = RuleId::from_static("cr.crux-unanswered");
/// A change request is to be resolved while parts of it are open.
pub const CR_PARENT_OPEN: RuleId = RuleId::from_static("cr.parent-open");
/// A crux is answered once.
pub const CR_CRUX_ANSWERED: RuleId = RuleId::from_static("cr.crux-answered");
/// A decision names an option the crux does not have.
pub const CR_CRUX_UNKNOWN_OPTION: RuleId = RuleId::from_static("cr.crux-unknown-option");
/// A duplicate or superseded resolution points at a change request that does not exist.
pub const CR_REFERENCE_UNKNOWN: RuleId = RuleId::from_static("cr.reference-unknown");
/// A resolution points at the change request it resolves.
pub const CR_REFERENCE_ITSELF: RuleId = RuleId::from_static("cr.reference-itself");
/// A crux is to supersede a crux that does not exist.
pub const CR_SUPERSEDES_UNKNOWN: RuleId = RuleId::from_static("cr.supersedes-unknown");
/// A crux is to supersede a crux that has no decision to replace.
pub const CR_SUPERSEDES_UNANSWERED: RuleId = RuleId::from_static("cr.supersedes-unanswered");
/// A crux is to supersede a decision that another crux replaces already.
pub const CR_SUPERSEDES_SUPERSEDED: RuleId = RuleId::from_static("cr.supersedes-superseded");
/// A decision rests on a policy line that does not exist.
pub const CR_POLICY_LINE_UNKNOWN: RuleId = RuleId::from_static("cr.policy-line-unknown");
/// A change request is titled as another is.
pub const CR_TITLE_TAKEN: RuleId = RuleId::from_static("cr.title-taken");
/// A crux is raised with no option to choose.
pub const CR_CRUX_NO_OPTION: RuleId = RuleId::from_static("cr.crux-no-option");
/// A crux is raised with the same choice twice.
pub const CR_CRUX_OPTION_TWICE: RuleId = RuleId::from_static("cr.crux-option-twice");
/// A crux is raised with more than one option recommended.
pub const CR_CRUX_RECOMMENDED_TWICE: RuleId = RuleId::from_static("cr.crux-recommended-twice");
/// A change request, or an option of a crux, names a target as a target and as kept, or twice.
pub const CR_TARGET_TWICE: RuleId = RuleId::from_static("cr.target-twice");
/// An option asks of a target the other kind than the change request, or a decision that stands, asks of it.
pub const CR_CRUX_EXPECT_CONFLICT: RuleId = RuleId::from_static("cr.crux-expect-conflict");
/// A crux of the kind `model` is left unspecified.
pub const CR_CRUX_UNSPECIFIED_NOT_ALLOWED: RuleId =
    RuleId::from_static("cr.crux-unspecified-not-allowed");
/// The option chosen names targets outside the change request, and the answer says nothing of where they go.
pub const CR_CRUX_ANSWER_OUTSIDE: RuleId = RuleId::from_static("cr.crux-answer-outside");
/// A target outside the change request is in the frame of none of the change requests the answer names.
pub const CR_CRUX_BEYOND_UNFRAMED: RuleId = RuleId::from_static("cr.crux-beyond-unframed");
/// The answer names the change request it was given in for what lies outside it.
pub const CR_CRUX_BEYOND_ITSELF: RuleId = RuleId::from_static("cr.crux-beyond-itself");
/// A withdrawal says where what lies outside goes.
pub const CR_CRUX_BEYOND_WITHDRAWN: RuleId = RuleId::from_static("cr.crux-beyond-withdrawn");
/// A Case cites an Item or a Clause it cannot reach.
pub const PROJECT_CITE_UNREACHABLE: RuleId = RuleId::from_static("project.cite-unreachable");
/// A sim Harness builds an Interface it cannot reach.
pub const PROJECT_BUILD_UNREACHABLE: RuleId = RuleId::from_static("project.build-unreachable");
/// A Context has more than one sim Harness.
pub const SIM_ONE_PER_CONTEXT: RuleId = RuleId::from_static("sim.one-per-context");
/// A sim Harness has no Case named `sweep`, or more than one.
pub const SIM_SWEEP_ONCE: RuleId = RuleId::from_static("sim.sweep-once");
/// A Case of a sim Harness is not named `sweep` or `regress-<name>`, or repeats a regress name.
pub const SIM_CASE_NAME: RuleId = RuleId::from_static("sim.case-name");
/// A sim Case cites no Clause.
pub const SIM_CASE_CITES_NONE: RuleId = RuleId::from_static("sim.case-cites-none");
/// A sim Case cites a Clause that no Contract of its Context has.
pub const SIM_CLAUSE_FOREIGN: RuleId = RuleId::from_static("sim.clause-foreign");
/// A sim Harness builds no Interface.
pub const SIM_BUILDS_NONE: RuleId = RuleId::from_static("sim.builds-none");
/// A sim Case cites an Item.
pub const SIM_CASE_CITES_ITEM: RuleId = RuleId::from_static("sim.case-cites-item");
/// A Harness of tests names Interfaces to build.
pub const SIM_BUILDS_OF_TESTS: RuleId = RuleId::from_static("sim.builds-of-tests");
/// A retirement would take away what Cases that stay cite.
pub const PROJECT_RETIRE_CITED: RuleId = RuleId::from_static("project.retire-cited");
/// A retirement would take away a Context that others use.
pub const PROJECT_RETIRE_USED: RuleId = RuleId::from_static("project.retire-used");
/// A Context is not in the Project.
pub const PROJECT_UNKNOWN_CONTEXT: RuleId = RuleId::from_static("project.unknown-context");
/// An asset was retired and its name is reserved.
pub const PROJECT_ASSET_RETIRED: RuleId = RuleId::from_static("project.asset-retired");
/// There is no project to work on.
pub const PROJECT_NONE: RuleId = RuleId::from_static("project.none");
/// A store holds one Project.
pub const PROJECT_EXISTS: RuleId = RuleId::from_static("project.exists");
/// A change set names one thing twice.
pub const CHANGE_SET_NAMED_TWICE: RuleId = RuleId::from_static("change-set.named-twice");
/// A Context based on its Contracts keeps one.
pub const CONTEXT_LAST_CONTRACT: RuleId = RuleId::from_static("context.last-contract");
/// A Context is based on its Contracts once.
pub const CONTEXT_ALREADY_BASED: RuleId = RuleId::from_static("context.already-based");
/// A Context has no asset with this id.
pub const CONTEXT_NO_SUCH_ASSET: RuleId = RuleId::from_static("context.no-such-asset");
/// The code of an Interface is pinned only when a Case checks it.
pub const PIN_UNCHECKED: RuleId = RuleId::from_static("pin.unchecked");
/// Two Items of an Interface have one key.
pub const INTERFACE_KEY_TAKEN: RuleId = RuleId::from_static("interface.key-taken");
/// An Item of an Interface is on lines with nothing in them.
pub const INTERFACE_BLANK_ITEM: RuleId = RuleId::from_static("interface.blank-item");
/// An Item of an Interface is on lines its code does not have.
pub const INTERFACE_LINES_OUTSIDE: RuleId = RuleId::from_static("interface.lines-outside");
/// A Case of a Harness is on lines its code does not have.
pub const HARNESS_LINES_OUTSIDE: RuleId = RuleId::from_static("harness.lines-outside");
/// An Item replaces a key that the same put still gives to an Item.
pub const INPUT_REPLACES_KEPT: RuleId = RuleId::from_static("input.replaces-kept");
/// Two Items of a put replace one key, or two renames name one key.
pub const INPUT_RENAME_CONFLICT: RuleId = RuleId::from_static("input.rename-conflict");
/// An Interface is put with no items, and nothing says which language to find them by.
pub const INPUT_ITEMS_REQUIRED: RuleId = RuleId::from_static("input.items-required");
/// A rename names a key that is not there.
pub const INPUT_RENAME_UNKNOWN: RuleId = RuleId::from_static("input.rename-unknown");
/// An Interface is put with items and with renames.
pub const INPUT_RENAMES_WITH_ITEMS: RuleId = RuleId::from_static("input.renames-with-items");
/// An Interface is put with no items, and no provider is configured to find them.
pub const ITEMS_NO_PROVIDER: RuleId = RuleId::from_static("items.no-provider");
/// The provider of items answered what the protocol or the code does not allow.
pub const ITEMS_PROTOCOL: RuleId = RuleId::from_static("items.protocol");
/// The provider of items could not be run to its end.
pub const ITEMS_FAILED: RuleId = RuleId::from_static("items.failed");
/// An operation is not one chidev knows.
pub const INPUT_UNKNOWN_OP: RuleId = RuleId::from_static("input.unknown-op");
/// An operation has a field it does not take.
pub const INPUT_UNKNOWN_FIELD: RuleId = RuleId::from_static("input.unknown-field");
/// An operation lacks a field it needs.
pub const INPUT_MISSING_FIELD: RuleId = RuleId::from_static("input.missing-field");
/// A field has a value of another type.
pub const INPUT_WRONG_TYPE: RuleId = RuleId::from_static("input.wrong-type");
/// A name does not exist.
pub const INPUT_UNKNOWN_NAME: RuleId = RuleId::from_static("input.unknown-name");
/// A value is not well formed.
pub const INPUT_INVALID: RuleId = RuleId::from_static("input.invalid");
/// A view is asked for no section.
pub const INPUT_VIEW_SECTIONS_EMPTY: RuleId = RuleId::from_static("input.view-sections-empty");

static RULES: &[Rule] = &[
    Rule {
        id: CR_ALREADY_RESOLVED,
        kind: RuleKind::Invariant,
        summary: "A resolved change request takes no more crux, decisions (but a withdrawal), assumptions, work or resolutions.",
        help: "The change request was resolved, and its record is closed. If the work goes on, reopen it with `cr_reopen` (a reason, and who reopens it); the resolution it had stays in its history. If the question is a new one, open a new change request. A crux raised in a resolved change request that was never decided can still be withdrawn with `cr_crux_decide` (`withdrawn`). When the decision of a crux names a resolved change request in `beyond`, that one is the second subject: it takes no more work, so reopen it or name another.",
        message: "the change request {s0} is resolved already",
        values: &[],
        example: r#"{"op":"cr_crux_raise","in":"Round halves up","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"up"},{"choice":"to even"}]}"#,
    },
    Rule {
        id: CR_CRUX_UNANSWERED,
        kind: RuleKind::Invariant,
        summary: "A change request cannot be resolved, whatever the resolution, while a crux raised in it has no decision.",
        help: "A crux is a question only a decision maker can answer, and it stops the change request it was raised in. Decide it with `cr_crux_decide` (an option, the basis it rests on, and who decides), then resolve. A decision to leave it unspecified, or to withdraw the question because it is not needed any more, is a decision too.",
        message: "the change request {s0} cannot be resolved: a crux raised in it has no decision",
        values: &[],
        example: r#"{"op":"cr_resolve","title":"Round halves up","resolution":"done","reason":"shipped","by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_PARENT_OPEN,
        kind: RuleKind::Invariant,
        summary: "A change request cannot be resolved while a part of it is open.",
        help: "A part is a change request whose `parent` is this one. Resolve each part first (`cr_resolve`), then the parent. A part that is not needed is resolved as `wont_do`.",
        message: "the change request {s0} has {count} part(s) that are not resolved: {others}",
        values: &["count"],
        example: r#"{"op":"cr_resolve","title":"Parent","resolution":"done","reason":"all parts done","by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_CRUX_ANSWERED,
        kind: RuleKind::Invariant,
        summary: "A crux is answered once and never rewritten.",
        help: "An answer is an input from outside that cannot be reproduced, so it is kept as it was given. A different decision is a new crux that supersedes this one (`cr_crux_raise` with `supersedes` the id of this crux).",
        message: "the crux raised in the change request {s0} is answered already",
        values: &[],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","option":"018f6b1e-0000-7000-8000-000000000002","basis":{"arbitrary":"changed my mind"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_CRUX_UNKNOWN_OPTION,
        kind: RuleKind::Invariant,
        summary: "A decision names an option the crux has.",
        help: "A decision points at an option of the crux by its id, not by its text. Name an option the crux has: `cr_crux_raise` gave the id of each, and `view` (the section `cruxes`) and `status` list them.",
        message: "the option named for the crux raised in {s0} is not one of its options",
        values: &[],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","option":"018f6b1e-0000-7000-8000-0000000000ff","basis":{"arbitrary":"settled"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_REFERENCE_UNKNOWN,
        kind: RuleKind::Invariant,
        summary: "A duplicate or superseded resolution points at a change request that exists.",
        help: "Name the change request that it duplicates or that supersedes it by its title. If it has not been opened yet, open it first with `cr_open`.",
        message: "the change request that the resolution of {s0} points at does not exist",
        values: &[],
        example: r#"{"op":"cr_resolve","title":"Old","resolution":{"duplicate":"No such title"},"reason":"same","by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_REFERENCE_ITSELF,
        kind: RuleKind::Invariant,
        summary: "A change request is not resolved by pointing at itself.",
        help: "Point at the other change request that duplicates it or supersedes it.",
        message: "the change request {s0} cannot be resolved by pointing at itself",
        values: &[],
        example: r#"{"op":"cr_resolve","title":"A","resolution":{"duplicate":"A"},"reason":"same","by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_SUPERSEDES_UNKNOWN,
        kind: RuleKind::Invariant,
        summary: "A crux supersedes a crux that exists.",
        help: "Name the crux whose decision the new one replaces, by its id (`view` and `status` list the cruxes).",
        message: "the crux that the crux raised in {s0} supersedes does not exist",
        values: &[],
        example: r#"{"op":"cr_crux_raise","in":"Redecide","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"a"}],"supersedes":"018f6b1e-0000-7000-8000-0000000000ee"}"#,
    },
    Rule {
        id: CR_SUPERSEDES_UNANSWERED,
        kind: RuleKind::Invariant,
        summary: "A crux supersedes a crux that has a decision to replace; a withdrawn crux has none.",
        help: "A decision is replaced by a new crux that points at it. A crux that is still open has no decision yet: decide it instead. A crux that was withdrawn decided nothing, so there is nothing in it to replace: raise a new question without `supersedes`.",
        message: "the crux that the crux raised in {s0} supersedes has no decision to replace",
        values: &[],
        example: r#"{"op":"cr_crux_raise","in":"Redecide","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"a"}],"supersedes":"018f6b1e-0000-7000-8000-000000000001"}"#,
    },
    Rule {
        id: CR_SUPERSEDES_SUPERSEDED,
        kind: RuleKind::Invariant,
        summary: "A decision is replaced by one crux; a crux supersedes a decision no other crux replaces.",
        help: "One decision stands for a question, so two cruxes do not replace the same one. `values.by` is the crux that replaces it already. To change the decision again, supersede the one that stands: the last crux of the chain (`view` and `status` list the cruxes with the `supersedes` of each). If it has no decision yet, decide it instead. A crux that was withdrawn replaced nothing, so the decision it pointed at may be superseded anew.",
        message: "the decision that the crux raised in {s0} supersedes is replaced by the crux {by} already",
        values: &["by"],
        example: r#"{"op":"cr_crux_raise","in":"Redecide","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"a"}],"supersedes":"018f6b1e-0000-7000-8000-000000000001"}"#,
    },
    Rule {
        id: CR_POLICY_LINE_UNKNOWN,
        kind: RuleKind::Invariant,
        summary: "A decision rests on a line the Project's policy has.",
        help: "Put the line in the policy first (`policy_put` replaces the lines whole), or decide with an arbitrary reason instead of a policy line.",
        message: "the decision of {s0} rests on the policy line {s1}, which the policy does not have",
        values: &[],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","option":"018f6b1e-0000-7000-8000-000000000002","basis":{"policy":"P9"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_TITLE_TAKEN,
        kind: RuleKind::Invariant,
        summary: "A change request is addressed by its title, so a title is given once.",
        help: "Use another title, or work on the change request that has it.",
        message: "a change request is titled {s0} already",
        values: &[],
        example: r#"{"op":"cr_open","title":"Round halves up","intent":"change_behavior"}"#,
    },
    Rule {
        id: CR_CRUX_NO_OPTION,
        kind: RuleKind::Invariant,
        summary: "A crux offers at least one option for the decision maker to choose.",
        help: "A crux is a question with answers to choose from, and a decision points at one of them by its id. Raise it again with `cr_crux_raise` and an `options` list that has an entry with a `choice`. A question that has no answers to offer is not a crux: settle it as an assumption (`cr_assume`) if the repository or general knowledge answers it.",
        message: "the crux raised in {s0} offers no option",
        values: &[],
        example: r#"{"op":"cr_crux_raise","in":"Round halves up","question":"How is a half rounded?","kind":"ambiguity","options":[]}"#,
    },
    Rule {
        id: CR_CRUX_OPTION_TWICE,
        kind: RuleKind::Invariant,
        summary: "The options of a crux are different choices.",
        help: "Two options with one `choice` could not be told apart by the person who decides. `values.option` is the choice given twice. Raise the crux again with one option for it, or give the two options different choices.",
        message: "the crux raised in {s0} offers the option {option} twice",
        values: &["option"],
        example: r#"{"op":"cr_crux_raise","in":"Round halves up","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"up"},{"choice":"up"}]}"#,
    },
    Rule {
        id: CR_CRUX_RECOMMENDED_TWICE,
        kind: RuleKind::Input,
        summary: "At most one option of a crux is recommended.",
        help: "A recommendation says which one option to take, with a reason and a confidence, and a policy line is written against it. `values.count` is how many options have `recommended`. Keep `recommended` on the one option that is recommended and take it off the others.",
        message: "the crux raised in {s0} recommends {count} options; at most one is recommended",
        values: &["count"],
        example: r#"{"op":"cr_crux_raise","in":"Round halves up","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"up","recommended":{"reason":"the literal reading"}},{"choice":"to even","recommended":{"reason":"the fairer reading"}}]}"#,
    },
    Rule {
        id: CR_TARGET_TWICE,
        kind: RuleKind::Input,
        summary: "A target is named once by a change request, or by one option of a crux, as a target or as kept.",
        help: "A target is to be revised or retired, or kept, not both and not twice: `targets` and `keep` of `cr_open`, or of one option of `cr_crux_raise`, name it in one place. `values.target` is the target named twice, and `values.option` the choice of the option that names it, which is there only when an option does (not for `cr_open`). Take it out of one of the lists, or out of both when neither is meant.",
        message: "{target} is named more than once, as a target or as kept, in what is given for {s0}",
        values: &["target", "option"],
        example: r#"{"op":"cr_crux_raise","in":"Round halves up","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"up","targets":[{"clause":{"context":"core","contract":"rounding","label":"C1"}}],"keep":[{"clause":{"context":"core","contract":"rounding","label":"C1"}}]}]}"#,
    },
    Rule {
        id: CR_CRUX_EXPECT_CONFLICT,
        kind: RuleKind::Invariant,
        summary: "An option asks of a target what the change request and the decisions taken in it ask of it, or nothing of it.",
        help: "A target is revised or retired, or kept, not both. An option that keeps what the change request (or a decision taken in it) asks to be revised or retired, or the reverse, would break that once it is chosen. `values.option` is the choice of the option and `values.target` the target. This is read when the crux is raised, for each option, and again when an option is chosen. Take the target out of the option; or, to change a decision taken before, raise the crux with `supersedes` the crux of that decision, which takes back what the decision asked.",
        message: "the option {option}, of a crux raised in {s0}, asks {target} the other of being revised or retired and kept than the change request or a decision in it asks",
        values: &["option", "target"],
        example: r#"{"op":"cr_crux_raise","in":"Round halves up","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"keep C1","keep":[{"clause":{"context":"core","contract":"rounding","label":"C1"}}]}]}"#,
    },
    Rule {
        id: CR_CRUX_UNSPECIFIED_NOT_ALLOWED,
        kind: RuleKind::Invariant,
        summary: "A crux of the kind `model` is not left unspecified.",
        help: "A question about the words of the model (kind `model`) is the one exception to \"undefined is an answer\": without the word the model cannot be written. `values.crux` is the crux and `values.kind` its kind. Decide it with `cr_crux_decide` by choosing an option (`option`), by wording it (`wording`) or by withdrawing it (`withdrawn`) if the question is not needed any more.",
        message: "the crux {crux}, raised in {s0}, is a {kind} question and cannot be left unspecified",
        values: &["crux", "kind"],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","unspecified":[],"basis":{"arbitrary":"left free"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_CRUX_ANSWER_OUTSIDE,
        kind: RuleKind::Invariant,
        summary: "An answer says where what its option adds outside the change request goes.",
        help: "A change request is about the targets it was opened with (its frame), and a decision never widens it: the targets the option chosen names to be revised or retired that are outside the frame are not added to the request. The answer says where they go, in `beyond`: `{\"requests\":[<titles>]}`, the change requests opened for them (each is open and frames the targets outside), or `{\"declined\":\"<why>\"}` if they are not done. `values.option` is the choice of the option and `values.targets` the targets outside. Open the change request for them with `cr_open`, then decide again.",
        message: "the option {option}, chosen for a crux raised in {s0}, names targets outside the change request, and the answer does not say where they go: {targets}",
        values: &["option", "targets"],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","option":"018f6b1e-0000-7000-8000-000000000002","basis":{"arbitrary":"settled"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_CRUX_BEYOND_UNFRAMED,
        kind: RuleKind::Invariant,
        summary: "A change request an answer names for what lies outside frames it.",
        help: "`values.target` is a target the option chosen names that is outside the change request, and no change request in `beyond.requests` frames it (names it, or a container of it, as a target to revise or retire). Open a change request that names it with `cr_open` and name that one, or add it to `beyond.requests`; or decline it with `beyond.declined`.",
        message: "{target}, named by the option chosen for a crux raised in {s0}, is outside it and in the frame of none of the change requests the answer names",
        values: &["target"],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","option":"018f6b1e-0000-7000-8000-000000000002","beyond":{"requests":["Another request"]},"basis":{"arbitrary":"settled"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_CRUX_BEYOND_ITSELF,
        kind: RuleKind::Invariant,
        summary: "An answer sends what lies outside its change request to another change request.",
        help: "What lies outside a change request cannot be taken by the same one: its frame is what it was opened with. Open another change request for it with `cr_open` and name that one in `beyond.requests`, or decline it with `beyond.declined`.",
        message: "the answer to a crux raised in {s0} names {s0} for what lies outside it",
        values: &[],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","option":"018f6b1e-0000-7000-8000-000000000002","beyond":{"requests":["Round halves up"]},"basis":{"arbitrary":"settled"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: CR_CRUX_BEYOND_WITHDRAWN,
        kind: RuleKind::Invariant,
        summary: "A withdrawal says nothing of what lies outside the change request.",
        help: "A withdrawal ends the question and asks nothing of any target, so there is nothing outside to place. Leave `beyond` out.",
        message: "the withdrawal of a crux raised in {s0} cannot say where anything outside goes",
        values: &[],
        example: r#"{"op":"cr_crux_decide","crux":"018f6b1e-0000-7000-8000-000000000001","withdrawn":true,"beyond":{"declined":"nothing was asked"},"basis":{"arbitrary":"not needed"},"by":{"person":"alice"}}"#,
    },
    Rule {
        id: PROJECT_CITE_UNREACHABLE,
        kind: RuleKind::Invariant,
        summary: "A Case cites only Items and Clauses its Context owns or uses.",
        help: "The cited item or clause is not in the Case's Context, nor in a Context it uses (it was removed, or the Context does not use the one that owns it). Put the Harness again without that citation, keep the item or clause (put it in the same change set), or make the Context use the one that owns it (`context_uses`).",
        message: "the {kind} {s1} is cited by {s0}, which cannot reach it from {s2}",
        values: &["kind"],
        example: r#"{"op":"harness_put","context":"core","name":"t","source":"fn t() {}","cases":[{"name":"t","lines":[1,1],"clauses":[{"contract":"other","label":"C9"}]}]}"#,
    },
    Rule {
        id: PROJECT_BUILD_UNREACHABLE,
        kind: RuleKind::Invariant,
        summary: "A sim Harness builds only Interfaces its Context owns or uses.",
        help: "Put the sim Harness again without that Interface in `builds`, keep the Interface, or make the Context use the one that owns it.",
        message: "the sim {s0} builds {s1}, which it cannot reach from {s2}",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":["gone"],"source":"","cases":[]}"#,
    },
    Rule {
        id: SIM_ONE_PER_CONTEXT,
        kind: RuleKind::Invariant,
        summary: "A Context has at most one sim Harness, so that a Clause is checked by one script.",
        help: "A simulation is one script per Context, which cites every Clause it checks. To replace the sim Harness, put the new one and retire the old one in the same change set (a change set checks the rules once, on the state it ends in). A Context that already has two, from before this was refused, takes a put of either only in a change set that retires the other.",
        message: "the context {s0} has more than one sim harness: {others}",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"sim-b","kind":"sim","builds":["api"],"source":"fn sim() {}\n","cases":[{"name":"sweep","lines":[1,1],"clauses":[{"contract":"rounding","label":"C1"}]}]}"#,
    },
    Rule {
        id: SIM_SWEEP_ONCE,
        kind: RuleKind::Invariant,
        summary: "A sim Harness has exactly one Case named `sweep`.",
        help: "A sim Harness is run case by case: the one Case named `sweep` stands for the seeded runs and cites every Clause the script checks. Name exactly one Case `sweep` (`count` is how many the Harness has), and put the Harness again.",
        message: "the sim {s0} has {count} case(s) named `sweep`; it has exactly one",
        values: &["count"],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":["api"],"source":"fn sim() {}\n","cases":[{"name":"regress-a","lines":[1,1],"clauses":[{"contract":"rounding","label":"C1"}]}]}"#,
    },
    Rule {
        id: SIM_CASE_NAME,
        kind: RuleKind::Invariant,
        summary: "A Case of a sim Harness is named `sweep` or `regress-<name>`, each regress name once.",
        help: "A sim Case is run under a name the protocol knows: `sweep` for the seeded runs, `regress-<name>` for a fixed schedule the script keeps (`<name>` is 1 to 64 of a-z, 0-9 and `-`, not starting with `-`). `reason` says which: a name that is neither, or a regress name that another Case has. Rename the Case, and put the Harness again.",
        message: "the name of {s0} is not one the sim {s1} can have: {reason}",
        values: &["reason"],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":["api"],"source":"fn sim() {}\n","cases":[{"name":"sweep","lines":[1,1],"clauses":[{"contract":"rounding","label":"C1"}]},{"name":"Stray","lines":[1,1],"clauses":[{"contract":"rounding","label":"C1"}]}]}"#,
    },
    Rule {
        id: SIM_CASE_CITES_NONE,
        kind: RuleKind::Invariant,
        summary: "A sim Case cites the Clauses it checks, at least one.",
        help: "A sim Case stands for the Clauses the script checks in that run, so that a Clause named in the script's report is traced to a Clause. Cite the Clauses it checks (`clauses`), and put the Harness again.",
        message: "{s0} of the sim {s1} cites no clause; a sim case cites the clauses it checks",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":["api"],"source":"fn sim() {}\n","cases":[{"name":"sweep","lines":[1,1]}]}"#,
    },
    Rule {
        id: SIM_CLAUSE_FOREIGN,
        kind: RuleKind::Invariant,
        summary: "A sim Case cites only Clauses of the Contracts of its own Context.",
        help: "The script names a Clause `<contract>:<label>` and is run against the Contracts of the Context its Harness is in. A Clause of a Contract of a Context it uses can be reached, but is not one of them. Cite the Clauses of this Context's Contracts, and put the Harness again, or put the sim Harness in the Context whose Contract it checks.",
        message: "{s0} cites the clause {s1}, which is in no contract of its context",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":["api"],"source":"fn sim() {}\n","cases":[{"name":"sweep","lines":[1,1],"clauses":[{"context":"base","contract":"other","label":"C1"}]}]}"#,
    },
    Rule {
        id: SIM_BUILDS_NONE,
        kind: RuleKind::Invariant,
        summary: "A sim Harness builds at least one Interface, the program its script drives.",
        help: "A simulation drives a program built from the pinned code of the Interfaces the Harness names in `builds`. Name the Interfaces to build, and put the Harness again.",
        message: "the sim {s0} builds no interface, so it has no program to drive",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":[],"source":"fn sim() {}\n","cases":[{"name":"sweep","lines":[1,1],"clauses":[{"contract":"rounding","label":"C1"}]}]}"#,
    },
    Rule {
        id: SIM_CASE_CITES_ITEM,
        kind: RuleKind::Invariant,
        summary: "A sim Case cites Clauses only, never an Item.",
        help: "A simulation checks Clauses. The Interfaces whose pinned code the script drives are the ones the Harness names in `builds`; naming one is not citing it, and puts the Harness on no Pin's basis. Cite Clauses only, and name the Interfaces in `builds`.",
        message: "{s0} of a sim harness cites an item; a sim case cites clauses only",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"sim","kind":"sim","builds":["api"],"source":"fn sim() {}\n","cases":[{"name":"sweep","lines":[1,1],"items":[{"interface":"api","key":"price"}]}]}"#,
    },
    Rule {
        id: SIM_BUILDS_OF_TESTS,
        kind: RuleKind::Invariant,
        summary: "Only a sim Harness names Interfaces to build.",
        help: "A Harness of tests checks the code of the Interfaces its Cases cite. Only a sim Harness builds the program it drives. Remove `builds`, or put the Harness with `kind` `sim`.",
        message: "{s0} is a harness of tests and names interfaces to build",
        values: &[],
        example: r#"{"op":"harness_put","context":"core","name":"tests","builds":["api"],"source":"fn t() {}\n","cases":[{"name":"t","lines":[1,1],"items":[{"interface":"api","key":"price"}]}]}"#,
    },
    Rule {
        id: PROJECT_RETIRE_CITED,
        kind: RuleKind::Invariant,
        summary: "Retiring does not take away what Cases that stay still cite.",
        help: "Retire the Harness of those Cases first, in the same change set (a change set checks the rules once, on the state it ends in), or put the Harness again without the citation. The peers are refused, not cascaded: what refers is retired first.",
        message: "cannot retire: {subjects} cite {cited}",
        values: &["cited"],
        example: r#"{"op":"contract_retire","context":"core","contract":"rounding"}"#,
    },
    Rule {
        id: PROJECT_RETIRE_USED,
        kind: RuleKind::Invariant,
        summary: "A Context that others use is not retired.",
        help: "Change the uses of the Contexts that use it (`context_uses`), or retire them too, in the same change set.",
        message: "cannot retire {s0}: {others} use it",
        values: &[],
        example: r#"{"op":"context_retire","context":"base"}"#,
    },
    Rule {
        id: PROJECT_UNKNOWN_CONTEXT,
        kind: RuleKind::Invariant,
        summary: "A Context an operation is about is in the Project.",
        help: "Add the Context first (`context_add`), or name one the Project has.",
        message: "{s0} is not in the project",
        values: &[],
        example: r#"{"op":"context_uses","context":"gone","uses":[]}"#,
    },
    Rule {
        id: PROJECT_ASSET_RETIRED,
        kind: RuleKind::Invariant,
        summary: "A retired asset is out of the model; its name and id stay reserved.",
        help: "Use another name for a new asset. A retired one is not brought back.",
        message: "the {kind} {s1} of the context {s0} was retired; its name and id are reserved, use another name",
        values: &["kind"],
        example: r#"{"op":"contract_put","context":"core","name":"retired-one","title":"T","clauses":[]}"#,
    },
    Rule {
        id: PROJECT_NONE,
        kind: RuleKind::Invariant,
        summary: "An operation works on a Project, which `init` makes.",
        help: "Run `init` first.",
        message: "there is no project yet",
        values: &[],
        example: r#"{"op":"status"}"#,
    },
    Rule {
        id: PROJECT_EXISTS,
        kind: RuleKind::Invariant,
        summary: "A store holds one Project, which `init` makes once.",
        help: "The store has its Project already, and a run starts where the last one ended: leave `init` out. To start another Project, give another store: `--store`, or a `[store] path` (the person's `.chidev/chidev.local.toml` outranks `chidev.toml`).",
        message: "there is a project already",
        values: &[],
        example: r#"{"op":"init"}"#,
    },
    Rule {
        id: CHANGE_SET_NAMED_TWICE,
        kind: RuleKind::Invariant,
        summary: "A change set names one thing once: which end state it wants must be said.",
        help: "A thing is put once, or retired once, not both and not twice. Merge the entries, or drop the one that is not wanted.",
        message: "the change set names the {kind} {s0} twice",
        values: &["kind"],
        example: r#"{"op":"change_set","puts":[{"put":"contract","context":"core","name":"c","title":"T","clauses":[]}],"retires":[{"retire":"contract","context":"core","contract":"c"}]}"#,
    },
    Rule {
        id: CONTEXT_LAST_CONTRACT,
        kind: RuleKind::Invariant,
        summary: "A Context based on its Contracts keeps at least one.",
        help: "The basis of a Context only goes up: once it is based on its Contracts it cannot quietly stop owing one. Put another Contract in the same change set as the retirement, or keep this one.",
        message: "the context is based on its contracts, and this is its last one",
        values: &[],
        example: r#"{"op":"contract_retire","context":"core","contract":"only"}"#,
    },
    Rule {
        id: CONTEXT_ALREADY_BASED,
        kind: RuleKind::Invariant,
        summary: "A Context is based on its Contracts once.",
        help: "The Context is already based on its Contracts, and the basis has no way back, so there is nothing to change: leave `base_on_contract` out. `view` shows a Context's basis.",
        message: "the context is already based on its contracts",
        values: &[],
        example: r#"{"op":"base_on_contract","context":"core"}"#,
    },
    Rule {
        id: CONTEXT_NO_SUCH_ASSET,
        kind: RuleKind::Invariant,
        summary: "A Context changes only assets it has.",
        help: "The id names no asset of that kind in the Context. Read the model with `status` and use the names it shows.",
        message: "the context has no {kind} {s0}",
        values: &["kind"],
        example: r#"{"op":"harness_retire","context":"core","harness":"gone"}"#,
    },
    Rule {
        id: INTERFACE_KEY_TAKEN,
        kind: RuleKind::Invariant,
        summary: "The Items of an Interface each have a key of their own.",
        help: "An Item is matched by its key when the code is put again, so two Items with one key are one Item to the matching and would be given one id. Give each Item a key of its own (`status` shows the keys an Interface has), and put the Interface again.",
        message: "{s0} has a key that another item of its interface has",
        values: &[],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\nfn b();\n","items":[{"key":"a","lines":[1,1]},{"key":"a","lines":[2,2]}]}"#,
    },
    Rule {
        id: INTERFACE_BLANK_ITEM,
        kind: RuleKind::Invariant,
        summary: "An Item is on lines that hold a declaration.",
        help: "The declaration of an Item is the text of its lines, so lines with nothing in them declare nothing. Point `lines` at the declaration, or leave the Item out, and put the Interface again.",
        message: "{s0} is on lines with nothing in them, so it declares nothing",
        values: &[],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n\n","items":[{"key":"a","lines":[2,2]}]}"#,
    },
    Rule {
        id: INTERFACE_LINES_OUTSIDE,
        kind: RuleKind::Invariant,
        summary: "An Item is on lines of its Interface's code.",
        help: "`lines` are the first and last line of the Item's declaration in `source`, counted from 1, so they cannot pass the last line of it. Count them in the code as it is put (a `source_file` is read when the operation runs), and put the Interface again. When `items_rekey` is refused so, the lines came from the provider, not from you: the provider read the code wrongly.",
        message: "{s0} is on lines {first} to {last}, but the code ends at line {total}",
        values: &["first", "last", "total"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n","items":[{"key":"a","lines":[2,3]}]}"#,
    },
    Rule {
        id: HARNESS_LINES_OUTSIDE,
        kind: RuleKind::Invariant,
        summary: "A Case is on lines of its Harness's code.",
        help: "`lines` are the first and last line of the Case in `source` (its test, an attribute above it included, or its case of a simulation), counted from 1, so they cannot pass the last line of it. Count them in the code as it is put (a `source_file` is read when the operation runs), and put the Harness again.",
        message: "{s0} is on lines {first} to {last}, but the code ends at line {total}",
        values: &["first", "last", "total"],
        example: r##"{"op":"harness_put","context":"core","name":"tests","source":"#[test] fn t() {}\n","cases":[{"name":"t","lines":[2,3],"items":[{"interface":"api","key":"price"}]}]}"##,
    },
    Rule {
        id: PIN_UNCHECKED,
        kind: RuleKind::Invariant,
        summary: "Code is pinned only when a Case checks one of the Items of its Interface.",
        help: "Nothing checked the code, so it is not accepted. Put a Harness whose Case cites an Item of the Interface (`harness_put`), then pin again.",
        message: "no case cites an item of {s0}, so nothing checked the code to pin",
        values: &[],
        example: r#"{"op":"pin","context":"core","interface":"api","code":"fn a() {}"}"#,
    },
    Rule {
        id: INPUT_UNKNOWN_OP,
        kind: RuleKind::Input,
        summary: "An operation is named by an `op` chidev knows.",
        help: "Run `rules`, or read the README, for the operations. `did_you_mean` lists the closest names.",
        message: "no operation is named `{name}`",
        values: &["name"],
        example: r#"{"op":"cr_opn","title":"x"}"#,
    },
    Rule {
        id: INPUT_UNKNOWN_FIELD,
        kind: RuleKind::Input,
        summary: "An operation takes the fields it lists and no others.",
        help: "Remove the field, or fix its spelling (`did_you_mean` lists the closest of the fields the operation takes).",
        message: "the operation has no field `{name}`",
        values: &["name"],
        example: r#"{"op":"context_add","name":"core","usess":[]}"#,
    },
    Rule {
        id: INPUT_MISSING_FIELD,
        kind: RuleKind::Input,
        summary: "An operation has the fields it needs.",
        help: "Add the field. The README lists the fields of each operation.",
        message: "the operation lacks the field `{name}`",
        values: &["name"],
        example: r#"{"op":"context_add"}"#,
    },
    Rule {
        id: INPUT_WRONG_TYPE,
        kind: RuleKind::Input,
        summary: "A field has the type the operation reads (a string, a list, an object).",
        help: "Write the value in the type the field takes; the message says what was found and what was expected.",
        message: "a field has a value of the wrong type: {detail}",
        values: &["detail"],
        example: r#"{"op":"context_add","name":["core"]}"#,
    },
    Rule {
        id: INPUT_UNKNOWN_NAME,
        kind: RuleKind::Input,
        summary: "A name an operation uses (a Context, an asset, a clause label, an item key, a case, a change request title, a crux id) exists.",
        help: "Use a name the project has (`did_you_mean` lists the closest), or make the thing first: `suggestions` has the operation that does. A crux is named by its id, which `cr_crux_raise` gave and `view` and `status` list. A clause label that `replaces` or `remove` of a clause put names is one the contract must have: `field` says which of the two it is in, and `did_you_mean` lists the labels the contract has.",
        message: "no {kind} is named `{name}`",
        values: &["kind", "name", "field"],
        example: r#"{"op":"cr_crux_raise","in":"No such title","question":"How is a half rounded?","kind":"ambiguity","options":[{"choice":"up"}]}"#,
    },
    Rule {
        id: INPUT_INVALID,
        kind: RuleKind::Input,
        summary: "A value is well formed: a name, a label, a line range, a text of the shape the field takes.",
        help: "The message says what is wrong with the value; write it again.",
        message: "{detail}",
        values: &["detail"],
        example: r#"{"op":"context_add","name":""}"#,
    },
    Rule {
        id: INPUT_REPLACES_KEPT,
        kind: RuleKind::Input,
        summary: "An Item or a Clause replaces a key or a label that no Item or Clause of the same put has.",
        help: "`replaces` renames: the Item takes the id of the Item that had the key it replaces, and the old key goes. Another Item of the same put that still has that key would be matched to the same id. Remove `replaces` if the key stays, or give the other Item another key. A `renames` entry is the same for the Items a provider finds: its `from` must not be a key the provider found. The same holds for the Clauses of a contract put, where the key is a label and `field` is `label`.",
        message: "{s1} of {s0} replaces the key `{key}`, which another entry of the same put still has",
        values: &["key", "field"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\nfn b();\n","items":[{"key":"a","lines":[1,1],"replaces":"b"},{"key":"b","lines":[2,2]}]}"#,
    },
    Rule {
        id: INPUT_RENAME_CONFLICT,
        kind: RuleKind::Input,
        summary: "A key is replaced by one Item of a put at most, and an Item replaces one key at most. A label is named once in a put of Clauses.",
        help: "`replaces` hands the id of one Item to one Item. Two Items that replace the same key would both take its id. Say `replaces` on one of them, and give the other no `replaces` (it is a new Item). The same holds for `renames`, where the provider finds the Items: a `from` named twice would hand one id to two Items, and a `to` named twice would give one Item two ids. `side` says which of `replaces`, `from` and `to` holds the key. For the Clauses of a contract put the key is a label, and `side` is `replaces` (two Clauses replace it), `label` (two Clauses are given it) or `remove` (it is removed and also given, replaced or removed again).",
        message: "{s1} of {s0} has `{side}` set to the key `{key}` twice",
        values: &["key", "side"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn x();\nfn y();\n","items":[{"key":"x","lines":[1,1],"replaces":"a"},{"key":"y","lines":[2,2],"replaces":"a"}]}"#,
    },
    Rule {
        id: INPUT_ITEMS_REQUIRED,
        kind: RuleKind::Input,
        summary: "An Interface put with no items is laid out with a language, to find them by.",
        help: "`items` may be left out when a provider finds the Items in the code. Which provider is the language of the Interface in `[[measure.layout]]` of the project's settings (`lang`). Lay the Interface out with a `lang`, or write `items` (a key and the `lines` of each).",
        message: "the items of {s1} of {s0} are left out, and its layout names no language to find them by",
        values: &[],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n"}"#,
    },
    Rule {
        id: ITEMS_NO_PROVIDER,
        kind: RuleKind::Input,
        summary: "An Interface put with no items has a provider configured for its language.",
        help: "`items` may be left out when a command in `[measure.items]` of the project's settings finds them (`<lang> = [\"chidev\", \"provider\", \"items-rust\"]`). The command is read from the `[measure]` table, so a project without one cannot leave the items out. Add the command, or write `items` (a key and the `lines` of each).",
        message: "the items of {s1} of {s0} are left out, and no provider finds them: {reason}",
        values: &["lang", "reason"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n"}"#,
    },
    Rule {
        id: ITEMS_PROTOCOL,
        kind: RuleKind::Input,
        summary: "The provider of items answers as the protocol `chidev-items 1` says and as the code is.",
        help: "A reply is `{\"protocol\":1,\"items\":[{\"key\":..,\"lines\":[first,last]}]}`: a key that is not empty and is one line, and lines that are lines of the code. The provider's own diagnostics go to standard error. Nothing was put. Fix the provider, or write `items` and leave it out of this put.",
        message: "the provider of items for `{lang}` did not answer as the protocol and the code of {s1} of {s0} allow: {detail}",
        values: &["lang", "detail"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n"}"#,
    },
    Rule {
        id: ITEMS_FAILED,
        kind: RuleKind::Input,
        summary: "The provider of items runs to its end.",
        help: "The command of `[measure.items]` could not be found or started, exited with a failure, or did not answer within the `timeout` of `[measure]`. Nothing was put. Run the command by hand with a request on its standard input, or write `items` and leave it out of this put.",
        message: "the provider of items for `{lang}` could not be run for {s1} of {s0}: {detail}",
        values: &["lang", "detail"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n"}"#,
    },
    Rule {
        id: INPUT_RENAME_UNKNOWN,
        kind: RuleKind::Input,
        summary: "A rename names a key that the Interface had (`from`) and a key that the provider found (`to`).",
        help: "`renames` hands the id of the Item that had the key `from` to the Item the provider found with the key `to`. A `from` that the Interface did not have has no id to hand over, and a `to` that the provider did not find has no Item to take it. Nothing is ignored silently; read the keys from `status` (before) and from what the provider finds (after).",
        message: "{s1} of {s0} renames the key `{key}` (`{field}`), which is not there",
        values: &["key", "field"],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n","renames":[{"from":"x","to":"a"}]}"#,
    },
    Rule {
        id: INPUT_RENAMES_WITH_ITEMS,
        kind: RuleKind::Input,
        summary: "`renames` goes with the Items a provider finds, and `items` is the Items written.",
        help: "When `items` is given, nothing is found and `replaces` of an item is how a key is renamed. Leave `items` out to have the provider find them and to rename with `renames`, or keep `items` and use `replaces`.",
        message: "{s1} of {s0} is put with items and with renames",
        values: &[],
        example: r#"{"op":"interface_put","context":"core","name":"api","source":"fn a();\n","items":[{"key":"a","lines":[1,1]}],"renames":[{"from":"x","to":"a"}]}"#,
    },
    Rule {
        id: INPUT_VIEW_SECTIONS_EMPTY,
        kind: RuleKind::Input,
        summary: "A view is asked for at least one section of the model.",
        help: "`sections` lists the parts to read: `contracts`, `interfaces`, `harnesses` and `change_requests`. An empty list asks for nothing, and there is no default that reads every section: say which ones to read.",
        message: "the field `{name}` of the view names no section",
        values: &["name"],
        example: r#"{"op":"view","sections":[]}"#,
    },
    Rule {
        id: UNMAPPED,
        kind: RuleKind::Invariant,
        summary: "A refusal of the domain that has no rule of its own yet (a bug to be mapped).",
        help: "The domain refused the operation, and the variant of its error is not yet tied to a rule. `error` carries the debug text of the error. Report it: the variant should have an id, a help and remedies.",
        message: "{text} (the domain has no rule id for this refusal yet: {error})",
        values: &["error", "text"],
        example: r#"{"op":"context_add","name":"core"}"#,
    },
    Rule {
        id: APP_UNMAPPED,
        kind: RuleKind::Invariant,
        summary: "A refusal of the application that has no rule of its own yet (a bug to be mapped).",
        help: "The operation was refused, and the refusal is not yet tied to a rule. `error` carries its text. Report it.",
        message: "{error}",
        values: &["error"],
        example: r#"{"op":"measure","kind":"mutate","context":"core"}"#,
    },
];

/// Every rule that is not a check: the invariants and the input rules.
pub fn rules() -> &'static [Rule] {
    RULES
}

/// The rule that is not a check with this id.
pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id.as_str() == id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn rule_ids_are_unique_and_every_rule_is_an_error() {
        let ids: BTreeSet<&str> = rules().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), rules().len());
        assert!(rules().iter().all(|r| r.level() == Level::Error));
        assert!(rules().iter().all(|r| r.kind != RuleKind::Check));
    }

    #[test]
    fn every_template_hole_is_a_subject_a_reserved_word_or_a_value_the_rule_carries() {
        for r in rules() {
            let mut rest = r.message;
            while let Some(open) = rest.find('{') {
                let after = &rest[open + 1..];
                let close = after.find('}').expect("a hole is closed");
                let hole = &after[..close];
                let subject = hole
                    .strip_prefix('s')
                    .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty());
                assert!(
                    subject || matches!(hole, "subjects" | "others") || r.values.contains(&hole),
                    "{}: the hole {{{hole}}} is nothing the rule carries",
                    r.id
                );
                rest = &after[close + 1..];
            }
        }
    }

    #[test]
    fn a_rule_is_found_by_its_id() {
        assert_eq!(
            rule("cr.parent-open").map(|r| r.kind),
            Some(RuleKind::Invariant)
        );
        assert!(rule("no.such-rule").is_none());
    }
}
