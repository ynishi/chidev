# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Breaking: the Crux is a record of its own, with an id, not a part of a change
  request. It is raised in a change request (`raised_in`), optionally from
  findings; its options and its answer have ids, and a recommendation and a
  decision point at an option by its id, not by the text of its choice. A
  decision is required, and a decision can be a choice, a wording, "unspecified"
  or, new, a withdrawal; a Crux of kind `model` cannot be left unspecified. The
  authority to decide is managed outside chidev (see
  `docs/adr/0003-the-authority-to-decide-a-crux-is-managed-outside-chidev.md`):
  `by` is kept as declared and checked against nothing. A change request is
  stopped by the Cruxes raised in it, and the targets of the option chosen
  become its decided expectations when the Crux is decided (an expectation the
  request already asks with the same kind is not added twice, and one asked with
  another kind is refused). Each decision keeps its own entries, with its own
  revision, and a Crux that supersedes a decision taken in the same request takes
  that decision's expectations back first. The README's "Crux, Policy and
  Assumption" has the operations.
- Breaking: `cr_resolve` refuses every resolution (`done`, `wont_do`, `invalid`,
  a duplicate, a superseded one) while a Crux raised in the request has no
  decision, `cr.crux-unanswered`; before, only `done` was refused. A Crux cannot
  be raised in a resolved request, and a Crux of a resolved request takes only a
  withdrawal.
- Breaking: `cr.awaiting-decision` is a warning, not a note, and says a Crux has
  no decision whether or not the request it was raised in is resolved.
- Breaking: `view` is `chidev-view 4` (`chidev schema view`). A change request
  has no `crux` in it, and the Cruxes are the new section `cruxes`. What was
  saved before is stale once: an external result (the model it was sent carries
  the version), and a judged finding about a change request that has a parent or
  a Crux, or about the Project of a store that has either. The stamp of a change
  request covers the Cruxes raised in it, and the stamp over the Project covers
  the Cruxes.
- Breaking: the store's layout is 2 (`PRAGMA user_version`). A file of layout 1,
  which held the Crux inside its change request, is migrated once when it is
  opened, in one transaction: each Crux becomes a record of its own (its question
  is the request's title, its options are numbered in order, a recommendation and
  a decision that named an option by its text name its id, a `supersedes` names
  the Crux of that request) and the change request documents lose their Crux.
  The store's revision and the model's do not move. What cannot be moved (a text
  that is none of the options, a `supersedes` that names a request with no Crux)
  stops the opening with what did not match and leaves the file as it was. An
  older chidev refuses a file of layout 2, so copy the store before upgrading.
  See "The store's layout" in the README.
- In Rust (`chidev-core`, a public API change): `Crux` is the aggregate
  `domain::entity::crux::Crux`, with `CruxId`, `CruxOptionId` and `AnswerId`, and
  `CruxRepository`, which `Store` requires; `Changes` has `cruxes`.
  `ChangeRequest` has no `crux`, and `Decision` has `Withdrawn`.
  `App::decide_change_request` is `App::raise_crux` and `App::decide_crux`, and
  `EditTrigger` has `RaiseCrux` and `DecideCrux` (`DecideChangeRequest` stays
  so that a row written before can be read). `ModelSection` has `Cruxes`,
  `CheckView` has `cruxes`, and `ModelView::build` takes the Cruxes.
- `lines` of an Item, a Case or an Assumption's evidence given in another form
  than two line numbers (a list of another length, one number) is refused with
  words that say their form (`[first, last]`, `[n, n]` for one line), not
  serde's ("invalid length 1, expected a tuple of size 2"). The README says an
  evidence's `lines` are the first and last line.
- The guide (`docs/guide/PROCEDURE.md`) says, in step 4, that chidev keeps a
  Clause and not the example files it names: a request that changes what such a
  file holds rewords the Clause as well, saying what changed, so that its
  revision moves and the Cases that cite it are checked again.
- The guide (`docs/guide/PROCEDURE.md`) says what the designer does with a gap
  the Harness writer sends back (a question of step 2, answered as work `for`
  the part that holds it, a resolved part reopened first, while its parent is
  open); that the Harness writer reads from `view` the part each put is `for`
  and the Cruxes (`cruxes`) that stop a part; that a comment gaining or losing
  lines moves the Items after it;
  that `mutation.unmeasured` is a note that no run was made; and that a
  Context is based on its Contracts in the change set that puts its first
  Contract. The example's Crux is now how large a count grows; a line that is
  not text is settled by the requirements, an Assumption.

- The guide (`docs/guide/PROCEDURE.md`) lets a Case the implementer finds
  wrong against its Clause go back to the Harness writer with that Clause, who
  puts the corrected Harness `for` the same part (step 4 no longer reads as one
  put per part), and points to an external check at the `pin` moment with
  `gate = true` for refusing a `pin` while the tests fail, since chidev does not
  run them (step 5): the program tests the working tree, so the code is pinned
  with `code_file`, and its `reads` name at least one section.

- The guide (`docs/guide/PROCEDURE.md`) decides points a first version left to
  judgement: a question is an Assumption when the requirements' text, the
  repository or general knowledge settles it to one reading, and a Crux
  otherwise; a Crux is raised with `cr_crux_raise` when the question comes up,
  even in the middle of the design, in the request it came up in, which waits
  for the decision (it is not resolved until its Cruxes are decided, however it
  would end) while the chosen option's targets become its expectations and the
  work goes `for` it; the decision maker is a role that applies the Policy, an
  answering agent or a person, and its authority is managed outside chidev;
  an acceptance example is a Clause that names its files, checked by a Case that
  reads them, so the tests before `pin` check it; an Interface has one Item per
  declaration a caller uses (`main` for a program); and whoever writes the
  Harness is given only what the record holds (`view` of the Contracts,
  Interfaces, Harnesses, change requests and Cruxes), the designer in a fresh
  session.

- The guide (`docs/guide/PROCEDURE.md`) answers what two first versions run
  through it had to guess:
  - the requirements and their acceptance examples are files in the
    repository, cited by path; the designer puts what an example shows into the
    Contracts, so the blind Harness writer still reads the Contracts and
    Interfaces only;
  - until a Crux is answered, the Clause, the Cases and the code that would
    keep the answer wait; code written meanwhile is not the answer;
  - an option names in `targets` the assets that exist and that choosing it
    would change, and says in `consequence` what else it affects;
  - the request a Crux is raised in names what holds what its answer adds, the
    Contract once it is there;
  - a Context's Contracts, Interfaces and Harness are kept in one part unless
    there is a reason to split them, and each put of a Harness is applied for
    the part whose change it carries;
  - an Interface's `source` is written as a caller sees it, with what is not a
    declaration in the language's comments.
- The README names an option's `consequence` (what else choosing it affects)
  where it describes options and `status`'s `awaiting`.
- The guide (`docs/guide/PROCEDURE.md`) runs the first version of a Project
  through change requests like any other change, from right after `init`, so
  that its requirements, Assumptions and open questions (a Crux) are kept; a new
  section, "The first version", says what each request names, with an example
  that runs. Its "Never" rule now covers the first version too: a Context or an
  asset is added, changed or retired only as the work of a request.

- A put answers with what the put introduced, not with everything the checks find
  in the Project it leaves. The put hook runs the checks on the Project before the
  edit and on the Project after it, and a finding is introduced when the state
  before has no finding of the same rule and subject, or has one that stands on
  fewer grounds. A mandatory check refuses a put only for what it introduced; a
  finding the Project already had no longer refuses an edit that did not make it.
  The invocations recorded still count every finding of the run on the state
  after. The decision, and the measurement behind it (a first Contract answered
  with five `cite.uncited-clause` warnings, and the next put with the same five),
  are in `docs/adr/0001-edit-reports-introduced-confirm-judges-whole.md`.
- The checks that say the model is not finished or not settled no longer run at
  `put`: `cite.uncited-clause`, `cite.uncited-item`, `pin.unpinned` and
  `sim.uncited-clause` run at `pin`, `resolve` and `check`;
  `context.no-interface` and `context.no-harness` likewise; and
  `cr.awaiting-decision` and `cr.intent-mismatch` run at `resolve` and `check`.
  What the whole model lacks is said by `check`, `status`, `pin` and
  `cr_resolve`. The checks that say an edit made something stale or out of place
  (`cite.stale`, `pin.stale`, `cr.keep-moved`, `cr.out-of-frame`,
  `crux.policy-line-gone`) keep their put hook and fill in their grounds, the typed
  marks of what makes a finding true that tell one put's finding from the last
  one's. `check_list` shows the `kind` of each check (`change` or `state`) beside
  its `hooks`.
- External checks at `put` are held to what the put introduced when the saved
  result of the rule describes the model before the put (the same input stamp and
  configuration stamp): a finding with a subject that result already had neither
  refuses a gate nor is reported again. Without such a result every finding
  counts, and a failed run refuses a gate whatever was saved. `pin` and `resolve`
  are as before.
- In Rust (`chidev-core`, a public API change): `CheckView` is one state, so
  `CheckView.before` (read by no check) is removed and `CheckView.after` is renamed
  `project`; `Change { before, after }` holds the two states of an edit.
  `Finding.mentions` is replaced by `Finding::grounds()` (`Ground`, `LineState`) and
  `Finding::mentions()` is derived from the grounds; `Descriptor` gains `kind()`
  (`CheckKind`), read from its hooks: a check that runs at `put` is a change; `introduced(before, after)` is new; `Run.refusing` is removed
  and `Registry::refusing` decides what refuses; `ALL_HOOKS` is replaced by a
  confirmation set of hooks inside the builtin checks; `OutOfFrame` carries the
  change set each moved target came from; `Project::context_of`,
  `Project::context_of_target` and `contexts_in` are new.
- Every use case that changes what the store keeps is a trigger of one of four
  kinds, and is kept only after the checks its kind asks for (an edit, a
  confirmation, a question, or a record that runs none); see
  `docs/adr/0002-every-use-case-that-changes-state-is-a-trigger-and-one-door-keeps.md`.
  Operations that were kept with no check are now edits: `context_add`,
  `context_uses`, `base_on_contract`, `items_rekey`, `cr_open`, `cr_assume` and
  `cr_reopen`, and so are `cr_crux_raise` and `cr_crux_decide`, which replace
  `cr_decide`. Their results have `findings`, those the edit
  introduced; a mandatory check refuses them for those, and an external check
  registered as a gate at `edit` is asked about them and can refuse them.
  `items_rekey` now says the citations it made stale (`cite.stale`). `init`,
  `measure`, `simulate` and `finding_report` are records and run no check.
- The words for when a check runs are renamed. The key `hooks` is `moments` in a
  registration in `chidev.toml`, and in a judged rule file; the moment of an edit
  is `edit`, not `put`; `check_list`, `check_rules` and `explain` show `moments`;
  `check_rules` takes `moment`, not `hook`. The old key, argument and word are
  refused, and the message names the new one. A file that has `hooks` or `put`
  must be changed.
- The request an external program is sent is protocol 2: `hook` is `moment`, and
  `put` is `edit`. A reply of protocol 1 or 2 is accepted, and any other version
  is a failed run.
- The renamed words do not make a saved external result stale. The digest of a
  registration, and the configuration stamp kept with a result, are made of the
  words of version 0.1.0 (`hooks`, `put`) through a copy written for the digest:
  the form of the digest is fixed at 0.1.0's. Changing anything else a
  registration says still makes the result stale.
- In Rust (`chidev-core`, a public API change): `Hook` is `Moment` (`Moment::Put` is
  `Moment::Edit`; `Moment::parse` reads a word, `Moment::digest_key` is the word a
  digest counts it by) and `Descriptor.hooks` is `Descriptor.moments`.
  `Invocation.hook` is `Invocation.moment`, and `Invocation` has a `trigger`.
  `App::add_context`, `set_uses`, `base_on_contract`, `items_rekey`,
  `open_change_request`, `raise_crux`, `decide_crux`, `assume` and
  `reopen_change_request` return their value in `Checked`, with the findings.
  `Store::commit` takes a `Gated` (made only in `app::commit`) and not `Changes`.
  `ExternalCall.hook` is `ExternalCall.moment`, and `ExternalCall.protocol` is 2.
- The frame of a change request follows citations: a Case that a change set
  moved (made, changed or retired) is inside the frame when it cites a Clause or
  an Item that is, or is inside, a target the frame names, and one such citation
  is enough. Before, a Harness put `for` a part that named a Contract was
  reported `cr.out-of-frame` for the Cases that follow its Clauses, which were
  not inside the Contract. The citations are recorded in the footprint when the
  work is applied (a Case the set changed counts what it cited before the set
  and after it, a Case made counts after, a Case retired counts before), so a
  later put cannot move earlier work in or out of the frame; a footprint kept
  without them is judged by containment alone. The README's "Attribution and
  frame" and the guide's step 4 say so.
- `cr.out-of-frame` also runs at the `resolve` moment (its moments are `edit`,
  `resolve` and `check`), still a warning, and its help no longer offers to name
  the target in `targets` after the fact: it says to name the target when the
  request is opened, to apply the change set `for` the request it belongs to, or
  to take the change back.
- In Rust (`chidev-core`, a public API change): a `Footprint` has `citations`, a
  map from each Case it moved to the Items and Clauses that Case cites and the
  containers they lie in (`Citations`, `Footprint::with_citations`,
  `Footprint::cites_inside`); `moves_between` is `footprint_between` and returns
  the footprint; `out_of_frame` and `keep_moved` take
  `resolving: Option<ChangeRequestId>`; `CheckView` has `resolving`;
  `judged_open` is public. A footprint kept by the store carries its
  `citations` (left out when empty; the change request document stays at its
  schema, which is not released).

### Fixed

- A `change_set` `for` a request that moves nothing (the same puts applied again)
  no longer adds an empty footprint to the request's `work`: it is applied and
  answers `ok`, and the work holds only footprints that moved something. A
  footprint that moved nothing kept in a store before stays as it was.
- The checks of the `resolve` moment judge the request being resolved. They read
  the model with it already resolved, and `cr.keep-moved`, `cr.intent-mismatch`
  and `cr.out-of-frame` skip a resolved request, so `cr.keep-moved` and
  `cr.intent-mismatch`, which declare that moment, never spoke of the request
  being resolved. `cr_resolve` now names the request it resolves to the builtin
  checks (not to an external one: the external protocol is unchanged), and they
  judge it.

### Removed

- The operation `cr_decide` and the field `crux` of `cr_open`. A line that has
  one is refused (`input.unknown-op`, `input.unknown-field`), and `did_you_mean`
  does not name the new one. In place of the `crux` of `cr_open`, raise the
  question with `cr_crux_raise` in the request it stops (`in` is its title;
  `question` is new, the `kind`, `importance`, `options` and `supersedes` stay,
  `recommended` is a mark on an option, and `supersedes` is a Crux's id, not a
  request's title). In place of `cr_decide`, decide with `cr_crux_decide`: the
  Crux is named by its id (`crux`, not `title`), and an option by its id
  (`option`, not the text `choice`); `wording`, `unspecified`, `basis` and `by`
  are as they were.

### Added

- `cr.keep-targeted` (warning; moments `edit` and `check`) tells a keep that an
  unresolved change request's target breaks for sure when the two meet, not
  after the work, when `cr.keep-moved` can only report it: a request keeps a
  target and an unresolved request (itself included) names the same target to
  revise or retire, or a Clause, Item or Case of the Contract, Interface or
  Harness that is kept. It is about the request that keeps, and its message
  names the request that targets and the targets. The edit that makes them meet
  (`cr_open`, `cr_crux_decide`, `cr_reopen`) is told in its own answer. To fix
  it, resolve the wrong request as superseded by a new one with the right
  `keep`. In Rust, `Ground::Targeted` and `Mention::Targeted` are new variants
  of the public enums, `service::change_request::keep_targeted` returns the
  `KeepTargeted` and `Conflict` it is made of, and `Project::holder_of` gives
  the Contract, Interface or Harness that holds a Clause, an Item or a Case.
- `chidev schema external-call` and `chidev schema external-reply` print the
  JSON Schema of the request an external check is sent and of the reply it
  gives, derived from the types that write and read them; the lists of a
  reply's versions, levels and subject kinds are the ones it is read with.
  The README says what the shape cannot: a finding's `rule` is the one asked
  for, its `message` is not blank, its `subject` is in the model sent; and
  that an empty `reads` is refused.
- A change request in `view` has its `parent` (an id), left out when it has
  none; its `situation`, issuer and Assumptions are not in it. The Cruxes are
  the section `cruxes`, which `view` and the `reads` of an external check name:
  each with its id, the request it stops (`raised_in`), its findings, kind,
  question, importance, options (each with an id), recommendation, what it
  supersedes, its state and, once decided, the answer (an id, the decision, the
  basis, `by` and `at`).
- `cr_crux_raise` raises a Crux in a change request, from findings or from the
  text of the request, and answers with the ids of the Crux and of its options;
  `cr_crux_decide` decides it, once, with an option (by id), a wording,
  "unspecified" or a withdrawal (`withdrawn`), the basis and who decided. The
  decision `Withdrawn` ends a question that is not needed any more. The rule
  `cr.crux-unknown-option` refuses a decision that names an option the Crux does
  not have, which `explain` describes; it was refused before with no rule
  (`domain.unmapped`). A Crux that is withdrawn cannot be superseded
  (`cr.supersedes-unanswered`).
- A second `base_on_contract` for a Context already based on its Contracts is
  refused by a rule of its own, `context.already-based`, which `explain`
  describes; it was refused before with no rule (`domain.unmapped`).
- A second `init` on a store that has its Project is refused by a rule of its
  own, `project.exists`, which `explain` describes; it was refused before with
  no rule (`app.unmapped`).
- An Item or a Case whose `lines` pass the end of its code is refused by a rule
  of its own, `interface.lines-outside` or `harness.lines-outside`, which
  `explain` describes; it was refused before with no rule
  (`domain.unmapped`). The README says what a Case's `lines` are: the first and
  last line of its test, an attribute above it included, or of its case of a
  simulation.
- The record of the checks that were run keeps the use case that ran them: a
  `trigger` column on `check_invocations` (added to a database made before it,
  with no layout change; the rows already there have none). The moment of a row
  is still written `put` for an edit, as 0.1.0 wrote it, so a database stays
  readable to 0.1.0; either word reads as an edit. In Rust,
  `chidev_core::app::commit::Trigger` lists the use cases by kind, and an
  `Invocation` has `trigger: Option<Trigger>`.
- `{"op":"check","context":"<name>"}` answers with the findings about one Context:
  those about it and about what it holds, and those about a change request with
  a target it revises, retires or keeps in the Context. `outside` counts the
  findings left out (about the Project as a whole, about a change request that
  names nothing of the Context, or about something that is gone) and is in the
  answer only when a `context` is given. A name that is no Context's is
  `input.unknown-name`. In Rust, `App::check(Option<&str>)` returns
  `CheckResult { findings, outside }`.
- `check_list` entries have a `kind`.

## [0.1.0] - 2026-10-25

The first release. Everything below was written while chidev was unreleased; an
entry that says what something "was" or what "earlier versions" did describes a
state no release had.

### Added

- `{"op":"view","sections":[..],"contexts":[..]}` reads the part of the model that is
  asked for: the `sections` named (`contracts`, `interfaces`, `harnesses`,
  `change_requests`; at least one, and there is no default), of the Contexts named
  (every Context when `contexts` is left out, none when it is `[]`). The result has
  the `store_revision` it was read at and the `view`; a Context always comes with its
  `id`, `name`, `basis` and `uses`, and a section that was not asked for has no key.
  It is cut from the model a judged rule is given and an external check is sent, so
  the entries are the same. In Rust, `App::view(&ViewInput)` returns `ViewRead`
  (`store_revision`, `view: ModelSlice`); `ModelSection` (domain) names a section.
- `chidev schema [ops|view]` prints a JSON Schema (2020-12) derived from the types
  that read and write: `ops` for one line of `exec` (`title` `chidev-exec 1`, `$id`
  `urn:chidev:schema:ops:1`), `view` for the `view` of the operation of that name
  (`chidev-view 3`, `urn:chidev:schema:view:3`). Without a name it lists them. A schema is
  a necessary condition: a rule that sets fields against each other (exactly one of
  `source` and `source_file`) is not in it and is named when an operation breaks it.
  `schemars` is a new dependency of `chidev-core` and `chidev-cli`.
- The input rule `input.view-sections-empty`, for a `view` that names no section.
- `items` of an `interface_put` (and of an `interface` entry of a `change_set`) can
  be left out: a provider finds the Items in the code, under the new protocol
  `chidev-items 1` (a request `{protocol, source}`, a reply `{protocol, items:[{key,
  lines}]}`, no `kind`). The language is the `lang` of the Interface's
  `[[measure.layout]]` and the command is the new `[measure.items]` table
  (`rust = ["chidev", "provider", "items-rust"]`), read by the loader of
  `[measure.mutants]`; a layout `lang` is now valid when either has a command for it.
  The provider is asked before the change set is planned and before the store is
  read, so a provider that fails refuses the set and the apply stays a function of
  its input; a reply is checked against the code (lines inside it, a key that is
  not empty). With `items` written no provider is started. The new `renames`
  (`[{"from","to"}]`, only without `items`) hands the id of the Item that had the key
  `from` to the Item found under `to`, with no revision moving; only the keys named
  are renamed. In Rust, `App::with_items` takes the port `ports::items::ItemSource`
  (the process adapter is `ProcessRunner`), `InterfacePut::items` is an `Option`,
  `InterfacePut::renames` and `RenameInput` are new, and `App::put_interface_of`
  takes both. A project with no `[measure]` has no provider of items
  (`items.no-provider`), and a language with a command in `items` alone is stopped
  by `measure` naming that `mutants` has none.
- `chidev provider items-rust`, the Rust provider of `chidev-items 1`, in
  `chidev-provider-rust` (the new module `items`, a pure function of the request like
  the provider of mutants; `quote` is a new direct dependency of that crate, already
  in the graph under `syn`). It finds `fn`, `struct`, `enum`, `union`, `trait`,
  `type`, `const`, `static` and `macro_rules!` (key `name!`) items and the `fn`,
  `const` and `type` in an `impl` or a `trait`, and a declaration with no body
  (`fn price(total: u64) -> u64;`), whose `lines` run from the first token past the
  doc comments and attributes to the line with the `;` or the `{` of the body. A key
  is a name and a path: `name`, `mod::name`, `Type<Args>::name` (inherent `impl`),
  `Trait::name`, `<Type<Args> as Trait<Args>>::name` (trait `impl`). It is printed
  from the syntax of the code, not by `syn`'s `Display`, by two rules: a list has its
  `,` between the elements only (a trailing `,` is dropped; a tuple of one element
  keeps it) and a space stands between two words and nowhere else. **A key is
  therefore not written as rustdoc writes it** (`<Box<dyn Error>as From<&str>>::from`,
  `fn()->u8`): copy it from `status` or the model. Keys of declarations that differ
  only by namespace or `#[cfg]` are equal and are refused as `interface.key-taken`
  (put such an Interface with `items` written); the names of the generics of an
  `impl` are in the key, so renaming `T` to `U` in `impl<T> Foo<T>` is a new key
  (`renames` carries the id); two declarations on one line have lines that overlap.
  The keys of the mutants are made by another function and did not change.
- `{"op":"items_rekey","context":..,"interface":..}` gives the Items an Interface
  keeps the keys and lines its provider finds in the code the Interface keeps (not
  the working tree), with their ids and so their citations kept: the way from the
  keys of a store from before keys (the old signature) or of keys written by hand to
  the provider's. An Item and a declaration are paired when their lines overlap and
  each overlaps no other on the other side; the rest are listed in `unmatched` and
  stay. A new key alone moves no revision; an Item whose new lines declare something
  else goes to its next revision (and the Interface with it) and is listed in
  `revised`, beside `rekeyed` and the Interface's `revision`. It is refused with
  nothing changed on `interface.key-taken` and on the `items.*` rules; it is one
  operation of its own, kept as the store stood when it began, not a `change_set`.
  In Rust, `App::items_rekey` returns `Rekeyed`, and the domain has
  `Interface::rekey` and `Project::rekey_interface`.
- Six input rules, each with a diagnostic, for the Items of an Interface put
  without `items`: `input.items-required` (no layout, or no `lang`),
  `items.no-provider` (no command in `[measure.items]` for the language, or no
  `[measure]`), `items.protocol` (a reply that does not match the protocol or the
  code), `items.failed` (the command could not be run to its end, or timed out),
  `input.rename-unknown` (`from` is not a key the Interface had, or `to` not one the
  provider found), `input.renames-with-items` (`items` and `renames` together). The
  rules `input.rename-conflict` (now also a `from` or a `to` named twice, with the
  new value `side`) and `input.replaces-kept` (now also a `from` the provider still
  finds) cover `renames` too.
- Four rules for the Items of an Interface, each with a diagnostic: the invariants
  `interface.key-taken` (two Items have one key; checked before the id of the
  Items, so an Item given twice under one key is a key to mend, not a uuid) and
  `interface.blank-item` (an Item's lines hold nothing), and the input rules
  `input.replaces-kept` (an Item `replaces` a key that an Item of the same put has)
  and `input.rename-conflict` (two Items `replaces` one key). The app refuses the
  last two before it gives an id, naming the Context, the Interface and the key.
- Diagnostics: a refusal is a violation of a rule, as structure, from the domain to
  the output. One registry holds the rules: the checks (as before), the
  *invariants* an operation is refused for (`cr.already-resolved`,
  `cr.crux-unanswered`, `cr.parent-open`, `cr.crux-answered`,
  `cr.reference-unknown`, `cr.reference-itself`, `cr.supersedes-unknown`,
  `cr.supersedes-unanswered`, `cr.policy-line-unknown`, `cr.title-taken`,
  `project.cite-unreachable`, `project.build-unreachable`, `project.retire-cited`,
  `project.retire-used`, `project.unknown-context`, `project.asset-retired`,
  `project.none`, `change-set.named-twice`, `context.last-contract`,
  `context.no-such-asset`, `pin.unchecked`) and the *input* rules
  (`input.unknown-op`, `input.unknown-field`, `input.missing-field`,
  `input.wrong-type`, `input.unknown-name`, `input.invalid`). An invariant is always
  `error` and no policy lowers it. Each rule has an id, a summary, a help (why, and
  how to fix), the keys of the values it carries (a key is added, never removed) and
  a message template. The domain gives `Violation { rule, subjects, values,
  remedies }` (ids, typed values, and typed `Remedy`s each with an `Applicability`:
  `machine`, `review`, `placeholder`); the app gives `Diagnostic { code, level,
  message, subjects, field, values, help, suggestions, did_you_mean }` with the
  names the store has. A name that does not exist (a Context, an asset, a clause
  label, an item key, a case, a change request title) is `input.unknown-name` with
  the close names that do (a match that differs only in case first, then an edit
  distance of at most a third of the length, as rustc does) and, where it helps, the
  operation that makes it (`cr_open` for a change request title). Error variants
  that have no rule yet give `domain.unmapped` (or `app.unmapped`), which carries
  the text of the error; a test lists those variants so that the list can only
  shrink.
- `{"op":"rules"}` lists every rule (id, kind, level, summary) and
  `{"op":"explain","rule":"<id>"}` explains one (help, the keys of its values, an
  example operation that violates it).
- String-literal mutants in the bundled Rust provider: the operator class `str`
  gives each string literal in a function body two mutants, the text emptied
  (`{...}` placeholders kept, so a format string still builds) and `*` appended
  inside the quotes; the keys are `<fn>/str-empty/<n>/..` and `<fn>/str-marker/<n>/..`.
  Raw strings are mutated; byte and C strings, strings in attributes and in the
  arguments of compile-time macros (`include_str!`, `env!`, `concat!`, ...) are not.
  The new optional request field `operators` and the new optional `[measure]
  operators` setting select the classes (default: all, `str` included); the setting
  is part of the settings digest, so changing it makes earlier runs stale. Protocol
  version 1 is kept: the field is optional. A project that has measured before will
  see new `str` mutants, and survivors among them, the next time it measures.
- `interface_put`, `harness_put` and `pin`, and the `interface` and `harness`
  entries of a `change_set`, can take their text from a file: `source_file` in
  place of `source`, and `code_file` (of `pin`) in place of `code`. Exactly one of
  the two is given. The path is relative to the directory `chidev` runs in; an
  absolute path, a `..` part, and a missing, unreadable or non-UTF-8 file are
  input errors that name the path, and a change set names every entry that has
  one. The file is read at the CLI boundary (`ops.rs`) when the operation runs;
  the use cases and the domain still receive text, and the model keeps the text,
  not the path. `lines` of Items and Cases are unchanged. `chidev-cli` enables
  tokio's `fs` feature.
- A mutation run that did not look at every site says so. The measurement keeps
  the `planned` mutants the provider gave beside the `sites` it found and the
  mutants evaluated, and a run that ended (`ok`) with fewer mutants than sites,
  `max_mutants` having limited it, is *capped*. The new check `mutation.capped`
  (note; about the Interface; `evaluated N of M sites`) reports it; a run that
  stopped early (the budget) is still a failed run and is not capped. The results
  of `measure` and `status.measurements` show `sites`, `evaluated` and `capped`. A
  run that ended without a failure and with fewer mutants than it was given is
  refused as a measurement. The summary of a stored mutation run has `planned`; a
  store written before it is not read (no compatibility). `check_list` has 27
  checks.
- A sim case whose program could not be started is no longer a finding of every
  Clause. The protocol `chidev-sim 2` gains the report line `CANNOT_START <text>`,
  which a script prints (with `sim: findings=0` and no `FINDING`, `UNREACHED` or
  `UNDECIDED` line) when it could not start the program; the case is recorded as
  `cannot_start:<text>` with no findings, does not count towards *unreached*, and is
  not run again. When every case that was run could not start the program, the run
  is a failure of the new kind `no_start`, kept with its cases, with no second
  runs, and judged by `sim.error` alone; when only some did, `sim.error` names
  them. The test fixture script prints it when the program is not there.
- Resolution of a change request is an explicit decision with a record, and a
  resolved request can be reopened. `{"op":"cr_resolve","title":..,
  "resolution":"done"|"wont_do"|"invalid"|{"duplicate":"<title>"}|
  {"superseded":"<title>"},"reason":..,"by":{"person"|"role":..}}` records how the
  request ended, why, by whom and when (from the clock). Only explicit states refuse
  it: the request resolved already, `done` while the Crux has no answer, a
  `duplicate` or `superseded` that points at itself or at a request that does not
  exist, and a part (a request whose `parent` it is) not resolved. The revisions of
  the targets and the work recorded are not consulted any more. `{"op":"cr_reopen",
  "title":..,"reason":..,"by":..}` opens it again; the request keeps a history, a
  log of `resolved` and `reopened` steps that is only appended to (stored as
  `history` in its document, layout 1), and takes work, decisions and assumptions
  again. `status.change_requests` has `resolution` and `history` for each request.
- `status.change_requests` shows `situation` (kept with the request; it was stored
  but not shown), and for each request its `targets` with what its work did to each
  (`revised`, `retired` or `not touched yet`, read from the footprints of the request
  and its parts) and its `keep` with whether each moved.
- The checks `cr.resolved-without-work` (warning; hooks `resolve`, `check`: resolved
  as done, yet no change set applied for it or its parts moved anything) and
  `cr.targets-untouched` (note; hooks `resolve`, `check`: resolved as done, with
  work, yet a target it named was never touched by it or its parts). `check_list`
  has 26 checks.

- Mutation testing of the pinned code, end to end for Rust. `{"op":"measure",
  "kind":"mutate","context":..,"interface":..}` copies the project to
  `.chidev/measure/<run>/`, writes the code the Pin accepted and the Harness sources
  of the model over the copy (the working tree's own code is not read), builds and
  tests it as it is, asks a provider for mutants, and for each one builds (a failure
  is stillborn) and tests (killed, survived or timeout). The result is kept as a
  measurement with a stamp (the Pin's code digest, the revisions the Pin stands on,
  a digest of the settings and of the provider program), the mutants with site,
  operator and outcome, and the evidence of the commands. `pin` now keeps the code
  text beside the Pin (`pin_codes`); a Pin made before has none and has to be made
  again to be measured.
- `[measure]` in `checks.toml`: `build` and `test` commands, `[measure.mutants]`
  (language to provider command), `[[measure.layout]]` (the file of each Interface
  and Harness), and `copy_exclude`, `env`, `timeout`, `baseline_timeout`, `budget`,
  `max_mutants`, `seed`. See the README, "Measurements".
- The provider protocol `chidev-mutants 1` (JSON on stdin and stdout; a mutant is an
  edit `{key, line, col, operator, span, from, to}` that chidev checks against the
  source before applying), and the bundled Rust provider, a new crate
  `chidev-provider-rust` exposed as `chidev provider mutants-rust`. It is a pure
  function of the request, ported from ryo-synth's operator table and even
  sampling of sites; it skips `#[cfg(test)]` and `#[test]` code, and finds sites by
  token spans inside function bodies, so a one-line function is mutated too.
- Four builtin checks that read the record of measurements: `measure.failed`
  (warning), `mutation.unmeasured` (note), `mutation.stale` (warning) and
  `mutation.survived` (note, one per surviving mutant). `check_list` has 16 checks.
  `status.measurements` shows the latest run per Interface: stamp, counts, `outcome`
  and `state` (`current` or `stale`).
- Store: tables `measure_runs` and `measure_items` (appended; the newest per kind and
  Interface is current) and `pin_codes`, created when an older database is opened.
- The blind simulation, for any language. A Harness has a kind, `tests` (the default)
  or `sim`; `harness_put` takes an optional `kind` (an existing Harness keeps its kind
  when none is given, and a change of kind is a new revision). A sim Harness is one
  script with exactly one Case named `sweep` and any number named `regress-<name>`;
  every Case cites Items (so the Harness enters the Pin's basis) and Clauses.
  `{"op":"measure","kind":"simulate","context":..,"harness":..}` copies the project
  under `.chidev/measure/<run>/`, writes the pinned code of the Interfaces the Cases
  cite and the script over the copy, runs `build`, runs the script for `seed-0` to
  `seed-<seeds-1>` and every regress case, and then runs `seed-0` and up to `sample`
  cases with findings once more in a fresh copy. It keeps a measurement of kind
  `simulate`: per case its exit, findings, unreached Clauses, undecided lines and the
  digests of `schedule` and `trace` (taken by chidev), how each second run compared
  with the first, the evidence, and a stamp (the Harness, the Contracts and
  Interfaces its Cases cite, the pinned code, the settings), in one commit.
- The protocol `chidev-sim 1`: `<interpreter...> <script> <case> --out <dir> --
  <program argv...>`; the script writes `<dir>/schedule` and `<dir>/trace` and prints
  `FINDING <case> step <k> <ClauseLabel>: <text>`, `UNREACHED <ClauseLabel>: <text>`,
  `UNDECIDED <text>` and a last line `sim: findings=<n>`. A label the sweep does not
  cite or a malformed line is a protocol failure, read in `infra`. A different
  schedule on the second run is a fault of the script, a different trace of the
  program. See the README, "Simulation", which also says how a sim script is written
  (blind, from the Contracts and Interfaces only, citing every Clause it checks in the
  sweep, with no randomness but the seed).
- `[measure.sim.<context>]` in `checks.toml`: `interpreter`, `program` (with `{tree}`
  for the root of the copy), `seeds` (50), `sample` (6), `timeout` and `budget`. A
  layout entry for an Interface needs a `lang` only for mutation, and `test` is
  needed only for mutation, so a project that is only simulated needs neither.
- Placeholders in `[measure]`: `{run}` is the run's directory and `{tree}` the root of
  the copy (`rerun-tree` in the second copy of a sim), the same in every field that
  takes one. Only the value of an `env` variable and a word of `sim.<context>.program`
  do (the default `env` is `CARGO_TARGET_DIR = "{run}/target"`); any other `{...}`
  there is left as it is.
  - `{work}`, and a `{run}` or `{tree}` in any field that takes no placeholder, is
    refused as a configuration error naming the field, so every command stops until
    the `checks.toml` is rewritten.
  - Rewriting the table changes the settings digest, so the measurements and
    simulations kept before are stale; measure again.
- Eight builtin checks: `sim.shape-kept` (error, hook `check`),
  `sim.uncited-clause` (warning), `sim.unrun` (warning), `sim.stale` (warning),
  `sim.finding` (error, one per Clause, with the first case and step), `sim.unreached`
  (error, per Clause), `sim.nondeterministic` (error, mandatory, per Case) and
  `sim.error` (warning). `check_list` has 24 checks. `status.measurements` also lists
  the latest simulation per sim Harness, with a `sim` summary.
- Failure kind `no_build`: the program a simulation drives does not build.
- Retiring assets, the D of CRUD. `context_retire`, `contract_retire`,
  `interface_retire` and `harness_retire` take a Context or an asset out of the live
  model and record it in the Project's `retired` (kind, names, the revision it had
  and the revision of what held it when it went; the text is not kept). Ids are
  reserved for good: a retired id is never given again, and nothing brings a thing
  back. The parts go with the whole (a Context takes its assets, an asset its Clauses,
  Items or Cases), and a Clause, an Item or a Case that a `*_put` drops is recorded as
  retired too. The peers are not cascaded: a retirement is refused, naming the
  Cases, when a Case that stays cites what would go, and a Context is refused while
  another uses it. The Project's revision moves when a Context is retired, a
  Context's when one of its assets is. A Context based on its Contracts keeps one.
  Outside the Project, Pins, change requests and measurements keep their ids: the Pin
  of a retired Interface reads `gone` and `pin.stale` does not report it, a `remove` expectation of a retired target is
  met, and wherever chidev names a target the live model no longer has (`status`,
  errors, `unapplied`, findings, measurements) it names it from `retired` with
  `(retired)`. `status.retired` lists the records. See the README, "Retiring".
- Change sets. `{"op":"change_set","puts":[..],"retires":[..]}` puts Contracts,
  Interfaces and Harnesses (and `uses` / `base_on_contract` of a Context) and retires
  Contexts and assets as one transition of the Project: the domain `ChangeSet` and
  `Project::apply` apply it to a copy in the order the rules need, check the rules of
  the Project once on the state it ends in, and keep all or nothing. A Case can cite
  a Clause or an Item that the same set makes, and a Harness can be put without the
  Cases that cite what the set removes, which a series of single puts refuses at its
  first step. The put hook runs once, on the Project before and after, and the store
  takes one commit with the names of the assets made. A thing named twice in a set is
  refused, and a refused set names every problem. Each entry is tagged `put`
  (`contract`, `interface`, `harness`, `uses`, `base_on_contract`) or `retire`
  (`context`, `contract`, `interface`, `harness`). The single puts and retirements
  are change sets of one and answer as before. See the README, "Change sets".
- Attribution of work to a change request, and its frame. `change_set` takes `for`,
  the title of a change request that exists and is not resolved. The set records its
  footprint for the request in the same commit: the targets whose revision differs
  between the Project before and after (made, revised or gone, containers included),
  each with the containers it was in. It is kept in the request's document (`work`),
  so the layout stays 1; a database written before does not load. A `change` or
  `remove` is now met only by a move in the footprints of the request or of its parts
  (children, recursively): a target that moved in a set for another request, or in a
  put, a retire or a set without `for`, does not meet it (those are judged by
  revision only for `keep`, which guards against anyone). `cr_resolve` names the
  targets that moved but not under the request; `status.unapplied` lists them as
  `elsewhere`. The targets a request expects to `change` or `remove` are its frame
  (a container frames everything in it), and the new check `cr.out-of-frame`
  (warning; hooks `put`, `check`) reports an unresolved request whose work moved a
  target outside its frame or the frame of a request it is a part of (a parent only
  aggregates: with no frame it bounds nothing, with one it bounds its parts).
  `status.change_requests` has `work` (each footprint's `store_revision` and the
  targets it moved) and `out_of_frame` for each request. `check_list` has 25 checks.
  A change set can add a Context: the entry `{"put":"context","name":..,"uses":[..]}`
  (as `context_add`; applied first, so the same set can put the Context's first assets
  and cite into it; it may use a Context the set adds). A `change` of the Project is
  met by a set `for` the request that adds a Context. See the README, "Change
  requests".
- The README describes the operations that build the model (`init`, `context_add`,
  `context_uses`, `base_on_contract`, `contract_put`, `interface_put`, `harness_put`,
  `pin`, `status`) with their JSON.
- The Project's decision Policy: numbered lines `P1`, `P2`, ... each with a
  text, and a revision that moves when its lines change (each line also records
  the revision at which it got its text). The operation `policy_put` replaces
  all the lines; `status.policy` shows them. The builtin check
  `crux.policy-line-gone` (note; put, check) reports an answer that rests on a
  Policy line that is gone or was reworded after it was applied.
- Assumptions: `assumptions` on `cr_open`, and the operation `cr_assume`
  (`title`, `statement`, `reason`, `evidence`) appends one to a request that is
  not resolved. They only grow. Evidence is a place in the repository (a
  relative path without `..`, with optional `lines`) or general knowledge (a
  reference).
- A `Clock` port of the application with a system adapter (`infra::clock::SystemClock`)
  and a fixed clock for tests (`infra::clock::FixedClock`); the domain takes the
  time as the value `Timestamp` and never reads a clock. The CLI sets the system
  clock.
- Domain values `Confidence`, `Importance`, `Recommended`, `Answer`, `Basis`,
  `DecisionMaker`, `Timestamp`, `Policy`, `PolicyLineId`, `Assumption` and
  `Evidence`, and the errors `AlreadyAnswered`, `SupersedesUnknown`,
  `SupersedesUnanswered`, `UnknownPolicyLine` and `PolicyRevisionMismatch` of a
  change request.
- Every part of the asset tree can be a target: the Project, a Context, a
  Contract, an Interface, a Harness, a Clause, an Item and a Case. The Project
  and a Context have a revision. The Project's moves when a Context is added or
  removed; a Context's moves when one of its assets is added or removed and when
  its `uses` or its basis change (not when an asset's content does). So an
  addition is written as a change of what holds the new thing (`change` the
  Contract for a new Clause, the Context for a new Interface, the Project for a
  new Context), a removal as `remove`, and a Case can be changed.
- The check `cr.keep-moved` (warning; put, resolve, check) reports an unresolved
  change request that keeps a target which has moved. The check
  `cr.intent-mismatch` (note; put, check) reports a keep-behavior request that
  changes or removes a Clause, and a change-behavior request that changes or
  removes nothing, in itself or in any option of its Crux.
- `status.awaiting` lists the change requests whose Crux has no decision, and the
  builtin check `cr.awaiting-decision` (note; put, resolve, check) reports them.
  `cr.unapplied` now also covers the changes a decision asked.
- External check execution through an application port and a Unix process adapter,
  with project registration, bounded JSON communication, timeout and cancellation
  cleanup, advisory results and mandatory change gates.
- Explicit `check_run` operations, saved external result freshness in `status`,
  and atomic SQLite storage of results and execution evidence alongside changes.
- Change-request addresses accept every finding subject through the shared
  `SubjectInput`, with legacy CLI item/clause input retained and ambiguous or
  misspelled subject selectors rejected. Rust `FindingInput` callers migrate
  from item/clause fields to `subject`.
- Judged rule definition digests: `check_rules` gives each rule a
  `rule_digest` over its deciding fields (id, subject, question, true, false),
  and `finding_report` requires it back; a missing or mismatching digest is
  refused and replaces nothing. A changed deciding field makes saved judgments
  stale; editing summary, fix, hints, hooks or level does not. Old database
  rows hold a null digest and show as stale.
- `chidev-cli` installs the `chidev` command (`cargo install chidev-cli`); the
  crates carry keywords and categories for crates.io.

### Changed

- The project's settings are `chidev.toml` (was `checks.toml`), and the person's are
  `.chidev/chidev.local.toml` (was `.chidev/checks.local.toml`); the old names are not
  read, and a file under one of them in a directory on the way up to the root stops
  `chidev exec` with status 2 and the name to give it (`ProjectError::Renamed`). The
  project's root is the nearest directory, from the one `chidev` runs in and
  then each parent, with a `chidev.toml` file (it may be empty); with none anywhere up,
  `chidev exec` stops with exit status 2 before any operation, instead of taking the
  current directory. The settings, the judged rules and the files a `source_file` or a
  `code_file` names are read from that root. A new `[store]` table (`path`, relative to
  the root) says where the store is, in either file, the person's first; `--store`
  (relative to the current directory) still wins, and the default is
  `<root>/.chidev/chidev.db` rather than `.chidev/chidev.db` under the current
  directory. In Rust, `infra::project::find` reads both files once into a `Project`
  (`root`, `file`, `store`), and `policy::load`, `external::load` and `runner::load`
  take `&Project`; `policy::PROJECT_FILE` and `PERSON_FILE` are gone (`project::FILE`,
  `PERSON_FILE`), `PolicyFileError` is a struct (`path`, `message`), and
  `policy::parse` reads a text with only `[checks]`.
- The public face of the SDK, for external checks: `ExternalModelView` is a newtype of
  `ModelSlice` and no longer has public fields, and `ExternalModelView` and
  `ExternalCall` no longer implement `Deserialize` (a program reads the document in
  another process; they still implement `Serialize`, `Debug` and `Clone`). The
  `reads` of an `ExternalRule` are `Vec<ModelSection>` rather than `Vec<String>`, kept in
  the order and with the repeats written; a word that is not a section is refused
  when `checks.toml` is loaded, as before, now by `unknown section `..` in reads`.
  What a program receives on stdin and the `input_stamp` and `config_stamp` that are
  kept with its report are the same bytes and values as before, so no saved external
  run goes stale.
- An Item of an Interface is known by a **key**, not by its signature. The identity of an
  Item was a string a person wrote and had to write the same way every time; one
  space or one `;` different made a new Item (the old one retired, its citations
  gone). Now an Item has a `key` (`ItemKey`: not empty, one line, never read by
  the domain), is matched by it when the Interface is put again, and the declaration
  (the signature) is derived by the Interface from the code and the Item's `lines`
  (`Signature::of_lines`: those lines, each without the white space at its end, joined
  by a line break) and is no longer given or stored. `interface_put` and the
  `interface` entries of a `change_set` take items `{"key","lines","replaces"?}` (it was
  `signature`); `replaces` names the key an Item had before, to keep its id when it
  is renamed. A citation of an Item is `{"interface":..,"key":..}` (a Case, and the
  `item` of a subject, an address or a target); `status`, the model view, a retired
  Item and the messages name an Item by its key. An Item's revision goes to the
  next only when the declaration derived from its lines changes; a rename alone
  moves no revision of the Item or the Interface, so a Case that cites it by id stays
  current. `AppError::UnknownItem` carries `key`; `app::ItemInput` and `app::ItemRef`
  have `key` in place of `signature`; `domain::entity::interface::ItemFound` is
  `(ItemId, ItemKey, LineRange)`. There is no compatibility for the old fields (an
  `items` entry with `signature` is refused: no `key`).
- The store is schema 2: an Item is kept as `(id, key, lines, revision)`, with no
  signature. A Project of schema 1 is read, in one place (the decode of the Project), as
  schema 2: the key of an Item is its old signature, each line without the white
  space at its ends joined by one space; a signature an Interface has twice gets
  ` #2`, ` #3`, ... from the second, in the order of the Items, moved on to the
  least number that is not the key of another Item; a retired Item keeps its name
  by the same rule. No revision moves, so no Pin goes stale and no citation with
  it. An Item whose lines hold no declaration makes the document corrupt, naming the
  Interface and the Item. Pins and change requests have the same shape in both
  schemas. The first put after the conversion changes no revision if the content
  is the same.
- A put of a sim Harness that is not the shape the protocol runs is refused (it was
  kept, and `sim.shape` reported an `error` that refused nothing: only a mandatory
  check at `error`, an invariant or a gate refuses, and the level of any other check
  is what is shown). The shape is eight invariants, each with a diagnostic that
  names the Harness, the Case or the Clause: `sim.one-per-context`,
  `sim.sweep-once`, `sim.case-name`, `sim.case-cites-none`, `sim.clause-foreign`,
  `sim.builds-none`, and the two that were `domain.unmapped` before,
  `sim.case-cites-item` and `sim.builds-of-tests`. A message no longer carries the
  id of a Clause. The rules are judged on the state a change ends in, so a sim
  Harness is swapped by putting the new one and retiring the old one in one
  `change_set`; a `measure` of a sim Harness in a shape a put is refused for is
  refused with the same diagnostics. The check `sim.shape` is replaced by
  `sim.shape-kept` (`error`, hook `check` only, not mandatory): a store from before
  the rule is read as it is (a put of a sim Harness is held to the shape, nothing
  else is), and the check reports what it holds, once per Clause for a Clause of no
  Contract of the Context. A Context that holds two sim Harnesses takes a put of
  either only in a `change_set` that retires the other. **A policy that names
  `sim.shape` stops with `UnknownCheck` (no operation runs): write `sim.shape-kept`
  instead.** `ProjectError::SimShape` and `HarnessError::BuildsOfTests(HarnessId)` are
  new; `domain::service::sim::shape` is gone (`Harness::sim_problems` and
  `Context::sim_problems` give the typed problems).
- A refused operation prints `{"ok":false,"line":n,"diagnostics":[...]}`; the
  `error` string is gone (unreleased: no compatibility). Read the `code` of a
  diagnostic, or its `message`. A line that is not an operation (bad JSON, an
  unknown `op`, an unknown or missing field, a value of another type) is a
  diagnostic of an `input.*` rule, with the `field` it is about. A change refused by
  checks keeps `findings` (and `external`). A finding in a result is a diagnostic
  too: it keeps `rule`, `level`, `state`, `message`, `about`, `stale` and `judged`,
  and gains `code` (the rule), `subjects`, `values`, `help`, `suggestions` and
  `did_you_mean`.
- The app's errors change with it: `AppError::RetireCited`, `RetireUsed`,
  `NotReached`, `BuildNotReached`, `NamedTwice`, `NothingChecksIt` and
  `ChildrenNotResolved` are gone, in favour of `AppError::Violated` (a list of
  `Violation`s and the names of what they are about) and the domain's own errors;
  `UnknownItem` and `UnknownClause` carry the Context; `UnknownCase` replaces an
  `Input`; an operation on a change request is refused as `ChangeRequestOf { id,
  title, error }`. `App::diagnose` gives the `Diagnostic`s of an `AppError`.
  `ChangeRequestError`, `ProjectError`, `ContextError` and `PinError` give a
  `Violation` (`violation`, `violations`). `app::Found` has `help`.
- `change` and `remove` of `cr_open` (and of the options of a Crux) are replaced
  by `targets` (what the request asks to have revised or retired) and `keep`.
  Neither list gates resolving a request; they are its content. `cr_open` refuses a
  field it does not know (it silently ignored them before), `situation` and
  `issuer` included in what it keeps. A parent may have no targets and is resolved
  once its parts are. The frame of a request (the targets it names; a container
  frames everything in it) is its own: the frame of a parent no longer bounds its
  parts. The kinds of an expectation in the results of `cr_open`, `status.awaiting`
  and the model view are `target` and `keep`; the model view is version 3. Stored
  change requests have `history` instead of `resolved` and `target`/`keep` kinds;
  a database written before does not load (layout 1, no migration).
- The sim protocol is `chidev-sim 2` and a sim Harness covers several Contracts.
  A script names a Clause `<contract>:<label>` (`FINDING <case> step <k>
  <contract>:<label>: <text>`, `UNREACHED <contract>:<label>: <text>`; the Contract's
  name has no white space), because a label is unique only within a Contract. chidev
  resolves each name to the Clause's id, among the Clauses the sweep cites, when the
  run is recorded, and the measurement keeps Clause ids (the stored `findings` and
  `unreached` hold `clause`, not `label`); the rules read ids and no longer resolve a
  label against the model again. A name without a Contract, of a Contract the sweep
  cites nothing of, or of a Clause it does not cite, is a protocol failure. Version 1
  is not kept (no compatibility code; layout stays 1, so a database with a simulation
  recorded before does not load). The rule "the sweep cites each label once" is gone.
  `sim.uncited-clause` ranges only over the Contracts the sweep cites a Clause of;
  a Contract it cites nothing of is left to the tests.
- A sim Harness has `builds`, the Interfaces of its Context whose pinned code the
  program is built from (`harness_put` and `change_set` harness entries of kind `sim`;
  a name the Context has no Interface of, or a Harness of tests with `builds`, is
  refused, and an Interface cannot be retired while a sim builds it). It replaces
  citing Items just to get the code built: a sim Case now cites Clauses only (a sim
  Case that cites an Item is refused, a Case of tests still needs one), so a sim is on
  no Pin's basis and putting it never makes a Pin stale. The simulation's own basis
  is the Harness (script, Cases and `builds`), the Contracts it cites a Clause of and
  each built Interface with the digest of its pinned code. A sim Harness that builds
  no Interface is refused (`sim.builds-none`). The stored Harness document gains `builds` (left
  out when empty).
- Errors name things, not ids: a Case that cites what it cannot reach is
  `the case stock-tests reserve refuses (context `core`) cites the clause stock C7,
  which it cannot reach`, and a Pin refused because no Case cites an Item of the
  Interface says `the interface `stock``. A Case that cites several unreachable
  targets is now several problems of a change set, not the first only.
- The stored Project document carries `retired` (layout 1, no compatibility code): a
  database written before does not load.
- The process machinery of the external checks (spawn without a shell, process
  group, deadline, output caps) moved to `infra::process`, which the measure runner
  shares. External behaviour is unchanged; the runner also takes a `locate` that
  does not follow the executable's links, because `cargo` is a link to `rustup`,
  which picks what to run by the name it was started as.

- A Crux is a choice that can be derived neither from the repository and general
  knowledge (an Assumption) nor from the Project's decision Policy, so a decision
  maker picks; it has no `default`, and what depends on the answer waits.
  `recommended` is `{choice, reason?, confidence?}` with a `confidence` of 0 to
  100, and the Crux takes an `importance` (`low`, `normal`, `high`) and
  `supersedes` (the title of a change request whose answered Crux it replaces).
- The decision is an `Answer`: the decision (chosen, worded or unspecified, as
  before), its `basis` (a Policy line with the Policy revision, or an arbitrary
  reason), the decision maker (`role` or `person`, by name) and the time. An
  answer is immutable: `cr_decide` on an answered Crux is refused, and a
  different decision is a new change request whose Crux `supersedes` the old
  one. `cr_decide` takes `basis` (`{"policy":"P1"}` or `{"arbitrary":".."}`) and
  `by` (`{"role":".."}` or `{"person":".."}`), both required; a Policy basis has
  to name a line the Policy has.
- `status` shows each request's `answer` (decision, basis, `by`, `at`) in place
  of `decision`, its `supersedes` and its `assumptions`; `status.awaiting` shows
  the undecided Cruxes with `kind`, `options`, `recommended` (with `confidence`)
  and `importance`, and `status.policy` the revision and lines of the Policy.
  `app::App::decide_change_request` takes the basis and the decision maker, and
  needs a clock (`App::with_clock`).
- The stored documents change again (layout 1, unreleased, no migration; an
  older database does not load): a Project has a `policy`, a change request has
  `assumptions` and (now required) `addresses`, and a Crux has `recommended` with
  a `confidence`, `importance`, `supersedes` and its `answer` in place of the
  request's `decision` and the Crux's `default`. A document with a field of the
  older layout is refused by name.
- `check_list` has 12 checks.

- A change request says what it expects of its targets instead of listing
  `lines`: `expects`, each target with a kind (`change`, `remove` or `keep`) and
  the revision it had when the request opened. `cr_open` takes `change`,
  `remove` and `keep`, each a list of references, in place of `items` and
  `clauses`; it answers with `expects` (kind, the target by name, `recorded`)
  in place of `lines`. A request still keeps its settled expectations and its
  optional `crux` apart, so a settled change does not disappear when a question
  about a detail is decided with its default.
- A Crux option takes the same `change` / `remove` / `keep` lists in place of
  `items` / `clauses`; its exact new text is `wording` (it was `change`).
  `cr_decide` records the revision of each target the option chosen expects
  something of, a later decision replaces them, and `cr_resolve` needs them met
  too.
- `cr_resolve` also refuses while a target it keeps has moved, and while a
  request whose `parent` it is is not resolved; each has its own error that names
  what is wrong by name, as `status` does. `intent` is a record only: no rule of
  resolving reads it any more, so a keep-behavior request can keep a Clause and
  change an Item and resolve without a warning.
- `status.unapplied` reports `unchanged` and `gone` (to change) and
  `not_removed` (to remove); a moved `keep` is reported apart, under
  `status.keep_moved`. The model view is version 2: a change request's `lines`
  became `expects`.
- The stored documents change (layout 1, unreleased, no migration; an older
  database does not load, and says which field is missing): the Project and each
  Context have a `revision`; a change request has `expects` and
  `decided_expects` (with a kind) in place of `lines` and `decided_changes`; a
  Crux option has `expects` in place of `changes`.

### Removed

- The check `cr.unapplied`, `status.unapplied` and the refusals of `cr_resolve`
  that read revisions (`not changed`, `not removed`, `gone`, targets that moved
  but not under the request, a moved `keep`): a resolution is a decision, and the
  record of how the targets fared is `status.change_requests[].targets` and the
  checks above. The ancestor frame (a parent's frame bounding its parts) is gone
  too.
- The checks `cr.keep-behavior-changed` (replaced by `cr.keep-moved`, which
  reads the expectations, not the intent) and `cr.parent-resolved-early` (a
  parent can no longer be resolved while a part is open, so there is nothing for
  a check to find).
- The change request's `lines`, the `items` / `clauses` fields of `cr_open` and of
  a Crux option, and the `lines` of the model view.
- The Crux's `default` (and `CruxKind::allows_default`, `ChangeRequestError::DefaultNotAllowed`):
  a choice that can be derived is an Assumption or is settled by the Policy, and
  one that cannot waits for a decision maker. A later decision no longer replaces
  an earlier one (see `supersedes`). The compatibility read of a change request
  stored without `addresses` is gone with it.

[Unreleased]: https://github.com/ynishi/chidev/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ynishi/chidev/releases/tag/v0.1.0
