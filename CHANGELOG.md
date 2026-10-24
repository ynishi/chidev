# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- A change request keeps its lines and its Crux apart. Every line has to change
  before it resolves, whatever the Crux decides (`Route` is gone: a request has
  `lines` and an optional `crux`), so a settled change no longer disappears when
  a question about a detail is decided with its default. Each Crux option may
  name the `items` / `clauses` that must change when it is chosen; `cr_decide`
  records their revisions, a later decision replaces them, and `cr_resolve`
  needs them changed too. `cr_resolve` names what has not changed by name, as
  `status.unapplied` does, instead of raw ids. `cr_open` answers with the lines
  it recorded. The stored change-request document gains `decided_changes` and
  per-option `changes`; an older database does not load (unreleased, no
  migration).

### Added

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
