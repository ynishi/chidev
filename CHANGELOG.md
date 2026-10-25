# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- `[measure.sim.<context>]` in `checks.toml`: `interpreter`, `program` (with `{work}`
  for the root of the copy), `seeds` (50), `sample` (6), `timeout` and `budget`. A
  layout entry for an Interface needs a `lang` only for mutation, and `test` is
  needed only for mutation, so a project that is only simulated needs neither.
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

### Changed

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

### Added

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
