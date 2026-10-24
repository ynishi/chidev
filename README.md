# chidev

Contract, Interface and Harness as the assets of a codebase; the code is a cache pinned to them

## Crates

| crate | role |
|---|---|
| `chidev` | Umbrella: the name users depend on. Re-exports the core. |
| `chidev-core` | Core (SDK): `domain` (pure rules and types), `app` (use cases and their ports), `infra` (adapters with real I/O). |
| `chidev-cli` | The `chidev` command. |

The layer rules are in `arch-lint.toml` and run with `cargo test`. A crate or layer
that does not fit the project is deleted, together with its `arch-lint.toml` scope.

## External checks

Register a project's checker in `checks.toml`:

```toml
[external."ext.contract-consistency"]
command = "./tools/check-contract"
args = []
hooks = ["put", "check"]
timeout = 10
reads = ["contracts"]
gate = false

[external."ext.contract-consistency".params]
strict = true
```

The executable runs without an implicit shell, from the project's root (the CLI's
working directory). Bare command names resolve through `PATH`. Registration is
project-only; the person's `.chidev/checks.toml` can change policy, not register
programs. A program has the caller's permissions: this is not a sandbox.

`chidev exec` accepts an explicit `{"op":"check_run"}` operation to execute
`check`-hook programs and record their results. `status` and `check` never start
an external program; they show saved findings. `status.external` shows `unrun`,
`current`, or `stale`, comparing the supplied model sections and registration /
policy with the saved execution. `current` describes freshness, not success.
Results at `off` are omitted, and the program is not started. Scalar parameter
defaults are declared under the registration's `params`; existing `[checks]`
policy layers may override them with values of the same type.

On `put`, `pin`, and `resolve`, matching external hooks check the proposed model
before it is saved. Advisory findings are capped at warning. `gate = true` makes
every finding an error and refuses the change; execution and protocol failures
also refuse it. A gate cannot be disabled or lowered by policy. A refused
operation returns its findings and execution evidence without saving them or the
candidate change. Accepted changes save their external results and invocation
records in the same transaction. `check_run` saves its results even when a gate
finds an error, because it changes no asset. A concurrent write refuses the
commit, with no automatic process retry.

Programs receive one JSON document on stdin and return one on stdout. For
example, a response with no findings is `{"protocol":1,"findings":[]}`. Diagnostics
go to stderr. `reads` selects `contracts`, `interfaces`, `harnesses`, and/or
`change_requests`; unrequested sections are omitted. Context metadata is always
sent. A program can name only the requested rule and subjects present in that
restricted view. See the RustDoc of `app::ports::external` for the wire types,
and `infra::external` for process limits and platform support.

The recorded SHA-256 identifies the executable read before launch. It does not
cover script arguments, dependencies, or later filesystem changes; displaying a
saved result does not re-hash the executable or check the filesystem. The check
input consists of the selected model sections, not a snapshot of the working
tree. Re-run a check after changing inputs outside that model.

## Change requests

A change request lists the definitions it asks to change (`items`, `clauses`: its
lines) and may carry a `crux`, a question for a person. The two are independent:

- Every line has to change before the request resolves, whatever the Crux
  decides. A line does not wait for the answer.
- Each option of the Crux may name `items` / `clauses` (the same references,
  context required) that must change if that option is chosen. The default
  option normally names none. `cr_decide` records the revision each of them has
  at that moment; a later decision replaces it. Deciding with the person's own
  wording asks the lines only, and leaving it unspecified asks nothing more.
- `cr_resolve` needs the Crux decided, every line past the revision recorded, and
  every definition of the chosen option past the revision recorded. When some
  have not changed it says which, by name (`rounding C1`), and which are gone.

```json
{"op":"cr_open","title":"Refuse big discounts","intent":"change_behavior",
 "clauses":[{"context":"pricing","contract":"discount","label":"C1"}],
 "crux":{"kind":"ambiguity","default":{"choice":"reuse InvalidDiscount"},
  "options":[{"choice":"reuse InvalidDiscount"},
             {"choice":"new error","clauses":[{"context":"pricing","contract":"discount","label":"C2"}]}]}}
```

`cr_open` answers with the lines it recorded, so a wrong reference is seen at
once: `{"kind":"clause","context":"pricing","contract":"discount","label":"C1","revision":1}`
(an item has `interface` and `signature` instead of `contract` and `label`).
`status` lists a request whose Crux has no decision under `awaiting` (title and
kind of the Crux) as well as under `unapplied` when its lines have not changed,
and the check `cr.awaiting-decision` (a note) reports it. A database written
before options named their changes does not load; chidev is unreleased and there
is no migration.

## Change-request addresses

Each entry in `cr_open.addresses` names a rule and the subject of its finding.
It uses the same `subject` form as `finding_report`:

```json
[
  {"rule":"context.no-harness","subject":{"context":"pricing"}},
  {"rule":"pin.unpinned","subject":{"interface":{"context":"pricing","interface":"rounding"}}},
  {"rule":"cite.uncited-item","subject":{"item":{"context":"pricing","interface":"rounding","signature":"fn round()"}}},
  {"rule":"cite.uncited-clause","subject":{"clause":{"context":"pricing","contract":"totals","label":"C1"}}},
  {"rule":"judged.case-review","subject":{"case":{"context":"pricing","harness":"rounding-tests","name":"half rounds up"}}},
  {"rule":"cr.unapplied","subject":{"change_request":"change rounding"}},
  {"rule":"judged.project-review","subject":null}
]
```

Omitting `subject` also names the whole Project. The legacy address forms with
an `item` or `clause` directly beside `rule` remain accepted. Each address may
contain only one selector key (`subject`, `item`, or `clause`), even when a value
is null. Unknown address fields and unknown fields inside a subject are rejected.

Names must resolve when the change request opens. After that, the reference uses
stable ids and survives later changes to the subject. The rule need not be
registered or currently report a finding; its id must be well formed. Opening a
request does not itself run a checker or judge a rule.

For Rust callers, `FindingInput` uses `subject: Option<SubjectInput>` in place of
its former `item` / `clause` fields. Wrap those references in
`SubjectInput::Item` / `SubjectInput::Clause`; `app::SubjectInput` remains the
public re-export. See `app::subject` and `domain::check::FindingState` in the
RustDoc for subject resolution and how a finding's state follows its requests.

## Judged rule versions

When an agent reads `check_rules`, each rule includes a `rule_digest`. The agent
must return that exact value in its `finding_report`, including a report with no
findings. A missing, null or malformed digest is refused as an input error, and
one that differs from the current definition as a rule version mismatch; either
way the report replaces nothing and records no run. The result carries the saved
`rule_digest` and the freshness `state`.

The digest covers only the fields that decide the answer: `id`, `subject`,
`question`, `true` and `false` (see `JudgedRule::digest` in RustDoc). The
summary, fix, `when` hints, hooks and default level describe, guide, schedule or
weight the rule without changing what is a finding, so editing them, TOML
comments or formatting, or a Policy level override leaves saved judgments
current. Changing a deciding field makes the saved report and every finding
stale. An App keeps its Registry for its lifetime; the CLI reloads
configuration on each invocation.

Database rows written before digests existed hold a null `rule_digest`: they
stay readable and show as stale (`rule_digest: null`), since the definition they
were judged on cannot be established after the fact. A matching digest still
cannot make a report on an older model current.

Rust callers supply `ReportInput.rule_digest` (a `String`) from
`RuleToJudge.rule_digest`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
