# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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

- The checks `cr.keep-behavior-changed` (replaced by `cr.keep-moved`, which
  reads the expectations, not the intent) and `cr.parent-resolved-early` (a
  parent can no longer be resolved while a part is open, so there is nothing for
  a check to find).
- The change request's `lines`, the `items` / `clauses` fields of `cr_open` and of
  a Crux option, and the `lines` of the model view.
