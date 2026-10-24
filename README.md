# chidev

Contract, Interface and Harness as the assets of a codebase; the code is a cache pinned to them

## Crates

| crate | role |
|---|---|
| `chidev` | Umbrella: the name users depend on. Re-exports the core. |
| `chidev-core` | Core (SDK): `domain` (pure rules and types), `app` (use cases and their ports), `infra` (adapters with real I/O). |
| `chidev-cli` | The `chidev` command. |
| `chidev-provider-rust` | The Rust mutant provider (`chidev provider mutants-rust`): a pure function from a source text to edits. Depended on by the CLI only; the core reaches a provider as a process. |

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

## Measurements

A Harness that passes does not show it is strong. `measure` runs the pinned code
against changes of itself (mutants) and keeps what the Harness noticed:

```text
{"op":"measure","kind":"mutate","context":"inventory","interface":"stock"}
```

It is an operation of its own, never a hook: it takes as long as the project's
build and tests take, once per mutant. It runs the code the Pin accepted (`pin`
keeps the text beside the Pin) and the Harness sources as the model holds them
now, in a copy of the project made under `.chidev/measure/<run>/`, with a target
directory of its own; the project's files and build output are never touched, and
the runner removes the copy when it is done (a killed process can leave one; it is
safe to delete). The working tree's own copy of the code is not read, so a tree
that has moved on does not change what is measured. Symbolic links are not copied.

The run builds and tests the copy once as it is (the *baseline*; if that fails the
run is a failed measurement and no provider is asked), asks the provider for
mutants, and for each one applies the edit, builds (a build that fails is
*stillborn*), tests (a failure *killed* it, a pass lets it *survive*, a deadline is
a *timeout*) and puts the code back. The result is kept as a measurement: its
stamp (the Pin's code digest, the revisions the Pin stands on, a digest of the
settings and of the provider program), each mutant with its site, operator and
outcome, the evidence of the commands (command, executable digest, exit code,
time), and the failure if there was one. Measurements are appended; the newest per
kind and Interface is current, the older stay as history
(`measure_runs`, `measure_items`). A result is kept in one commit with nothing else.

The checks read the record and run nothing (they are builtin, at the `check` hook):

| rule | default | finds |
|---|---|---|
| `measure.failed` | warning | the latest run failed (baseline, provider, protocol, timeout, budget, io) |
| `mutation.unmeasured` | note | a pinned Interface with no run |
| `mutation.stale` | warning | the Pin's code, the Interface, a Harness that checks it, a Contract it cites, or the settings moved after the run |
| `mutation.survived` | note | one per mutant no case noticed, about the Interface, with key, line, column, operator and the change |

A stale run is judged stale first, so a survivor of an old run is not reported for
moved code. `status.measurements` lists the latest run per Interface: stamp,
`killed` / `survived` / `stillborn` / `timeout`, `outcome` and `state`
(`current` or `stale`).

### `[measure]` in `checks.toml`

```toml
[measure]
build = ["cargo", "build", "--quiet", "--tests"]   # a failure makes the mutant stillborn
test  = ["cargo", "test", "--quiet"]               # all pass: survived; a failure: killed
copy_exclude = [".git", "target", ".chidev"]       # relative to the root; the default
env = { CARGO_TARGET_DIR = "{work}/target" }       # default; {work} is the run's directory
timeout = 30            # seconds, one build or one test of a mutant
baseline_timeout = 600  # the build and the test before any change
budget = 900            # the whole run, at most 3600
max_mutants = 40
seed = 0                # moves where the even sampling of sites starts

[measure.mutants]       # language -> the provider command
rust = ["chidev", "provider", "mutants-rust"]

[[measure.layout]]      # where an Interface's code is in the tree
context = "inventory"
interface = "stock"
lang = "rust"           # selects the provider
path = "src/stock.rs"

[[measure.layout]]      # where a Harness's source is written
context = "inventory"
harness = "stock-tests"
path = "tests/stock.rs"
```

`build` and `test` are commands, run without a shell in the copy; paths are
relative to the project root and may not contain `..`. Every Harness that checks the
Interface (the Harnesses of the Pin's basis) needs a layout entry, and the Harness
must be a file of its own (an in-file `#[cfg(test)]` module cannot be told apart
from the code). A field that is not known is refused by name. A project with no
`[measure]` has no runner: `measure` is refused, and the checks still judge what is
kept. The settings digest covers the whole table and the provider's executable, so
changing a command or upgrading the provider makes earlier runs stale.

### Provider protocol `chidev-mutants 1`

The provider is a program that makes mutants and nothing else: a pure function
of a source text, so a provider for another language is small. One JSON
document on stdin, one on stdout, stderr for diagnostics:

```json
{"protocol":1,"path":"src/stock.rs","source":"...","max":40,"seed":0}
{"protocol":1,"sites":118,"mutants":[{"key":"reserve/rel/2/9f3a","line":42,"col":13,
  "operator":"rel","span":[1834,1835],"from":"<","to":"<="}]}
```

A mutant is an edit: `span` is a byte range of `source`, `from` the text it holds,
`to` what replaces it. chidev checks `source[span] == from` for every edit before
any is applied; a disagreement, a key given twice, a protocol other than 1, or a
field the protocol does not have is a failed run (`protocol`). `key` is the
provider's stable id of the mutant (the bundled one writes
`<function>/<operator>/<n>/<digest of the line>`, which an edit elsewhere in the
file does not move); chidev treats it as opaque text. The reply is capped at 4 MiB.
The bundled Rust provider is `chidev provider mutants-rust`: it mutates operators
(`rel`, `eq`, `arith`, `logic`), boolean literals (`bool`) and integer literals
(`int`) inside function bodies, leaves `#[cfg(test)]` and `#[test]` code alone, and
gives the same reply for the same request.

## Change requests

A change request says what it expects of the targets it points at, in three
lists, and may carry a `crux`, a choice a decision maker has to make, and the
`assumptions` made while raising it (see below). The targets are the assets, as a
tree, and each has a revision:

```text
Project
 └ Context
    ├ Contract  ─ Clause
    ├ Interface ─ Item
    └ Harness   ─ Case
```

A reference names a target: `{"project":true}`, `{"context":"pricing"}`,
`{"contract":{"context":"pricing","contract":"discount"}}`,
`{"interface":{"context":..,"interface":..}}`, `{"harness":{"context":..,"harness":..}}`,
`{"item":{"context":..,"interface":..,"signature":..}}`,
`{"clause":{"context":..,"contract":..,"label":..}}` and
`{"case":{"context":..,"harness":..,"name":..}}`. Every target has to exist when
the request opens, and a target has one expectation.

- `change`: the revision of the target goes past the one recorded at `cr_open`.
  Adding something is a change of what holds it: a Clause, `change` its Contract;
  a Contract, Interface or Harness, `change` its Context; a Context, `change`
  the Project. A Context's revision moves when one of its assets is added or
  removed, or its `uses` or basis change, not when an asset's content does; the
  Project's moves when a Context is added or removed.
- `remove`: the target is no longer there (a Context takes its assets with it).
- `keep`: the revision of the target is still the one recorded. A request cannot
  be resolved once a target it keeps has moved.
- Each option of the Crux takes the same `change` / `remove` / `keep` lists
  (and `wording`, the exact new text of the definitions, which was `change`
  before the lists took that name). `cr_decide` records the revision each of
  them has at that moment. Deciding with the decision maker's own wording, or
  leaving it unspecified, expects nothing more.
- `intent` (`change_behavior` or `keep_behavior`) is a record. No rule of
  resolving reads it; the note `cr.intent-mismatch` points out a keep-behavior
  request that changes or removes a Clause, and a change-behavior request that
  changes or removes nothing.
- `cr_resolve` needs the Crux answered, every `change` and `remove` met (the
  request's own and the chosen option's), no `keep` moved, and every request
  whose `parent` it is resolved. When it refuses it names what is missing, by
  name (`rounding C1`): what has not changed, what is gone, what is not removed
  yet, what moved, or which parts are open.

```json
{"op":"cr_open","title":"Refuse big discounts","intent":"change_behavior",
 "change":[{"clause":{"context":"pricing","contract":"discount","label":"C1"}}],
 "crux":{"kind":"ambiguity","importance":"normal",
  "recommended":{"choice":"reuse InvalidDiscount","reason":"no new type","confidence":85},
  "options":[{"choice":"reuse InvalidDiscount"},
             {"choice":"new error","change":[{"clause":{"context":"pricing","contract":"discount","label":"C2"}}]}]}}
```

```json
{"op":"cr_open","title":"Add billing and drop C2","intent":"change_behavior",
 "change":[{"project":true}],
 "remove":[{"clause":{"context":"pricing","contract":"discount","label":"C2"}}],
 "keep":[{"clause":{"context":"pricing","contract":"discount","label":"C1"}}]}
```

`cr_open` answers with the expectations it recorded, so a wrong reference is
seen at once: `{"kind":"keep","target":{"clause":{"context":"pricing","contract":"discount","label":"C1"}},"recorded":1}`.
`status` lists a request whose Crux has no decision under `awaiting` (title, kind,
options with what each expects, the recommendation with its confidence, and the
importance), the requests whose `change` / `remove` have not been met
under `unapplied` (`unchanged`, `gone`, `not_removed`), and those whose `keep`
moved under `keep_moved`. The checks `cr.awaiting-decision` (a note),
`cr.unapplied` and `cr.keep-moved` (warnings) report them. A "change" is judged
by the revision alone, so another request's change satisfies it too, and a change
to a container is met by any change inside it that moves it; tying a change to
the request that made it is left for later. A database written before
expectations, or before the Crux, Policy and Assumptions below, does not load;
chidev is unreleased and there is no migration.

### Crux, Policy and Assumption

Three things say how a question that came up while raising a request is settled,
by how it can be answered:

| | when | who | recorded |
|---|---|---|---|
| Assumption | there is no choice: the repository and general knowledge give the answer | the role at work | the statement, the reason, the evidence |
| Crux settled by the Policy | a choice must be made, and a line of the Policy derives it | a decision maker (a role such as `orchestrator`) | the choice, the Policy line applied and the Policy revision |
| Crux nobody derives | not even the Policy settles it | a person | the choice, the reason, who decided |

A **Crux** is a choice that cannot be derived, neither from the repository and
general knowledge (that is an Assumption) nor from the Project's decision Policy,
so a decision maker picks. It has no default: what depends on the answer waits,
and what does not goes on. Most choices are settled by the Policy and recorded as
such; what is left is an arbitrary, intentional decision, and the point is to
record who decided what, on which basis, strictly and immutably, because it
cannot be reproduced. It applies to every change request and finding (Contracts,
Interfaces, Harness implementation, code), not only to the specification.

- The Crux has a `kind`, its `options` (each with the expectations it asks), a
  `recommended` choice `{choice, reason?, confidence?}` where `confidence` is a
  percentage from 0 to 100, an `importance` (`low`, `normal` or `high`), and
  `supersedes`, the title of a change request whose answered Crux this one
  replaces (it has to exist and be answered when the request opens). The
  recommendation and the importance are what a Policy line is written against.
- `cr_decide` gives the answer, once:
  `{"op":"cr_decide","title":..,"choice":..` (or `"wording"`, or `"unspecified"`)
  `,"basis":{"policy":"P1"}` (or `{"arbitrary":"<reason>"}`)
  `,"by":{"role":"orchestrator"}` (or `{"person":"alice"}`)`}`. A `policy` basis
  has to name a line the Policy has now, and the revision of the Policy is
  recorded with it. The time comes from the clock of the application, never from
  the domain. An answer is never rewritten: deciding a Crux that has an answer is
  refused. Changing a decision means opening a new change request whose Crux
  `supersedes` the old one; the old answer stays as it was given.
- The **Policy** is the Project's decision policy: numbered lines `P1`, `P2`, ...
  each with a text. `policy_put` replaces all of them:
  `{"op":"policy_put","lines":[{"id":"P1","text":"take the recommendation when its confidence is 80 or more"}]}`.
  Its revision goes up when the lines change, and `status.policy` shows the
  revision and the lines. The note `crux.policy-line-gone` (hooks: put, check)
  reports an answer that rests on a line which is gone or whose text changed
  since; the answer itself stands.
- An **Assumption** `{statement, reason, evidence}` is an interpretation settled
  from the repository and general knowledge, so nobody is asked. `evidence` is a
  list of `{"repo":{"path":"src/round.rs","lines":[10,14]}}` (a path relative to
  the repository, without `..`; `lines` optional) and
  `{"knowledge":{"reference":"IEEE 754 roundTiesToEven"}}`. Give them with
  `cr_open.assumptions`, or append later with
  `{"op":"cr_assume","title":..,"statement":..,"reason":..,"evidence":[..]}`.
  Assumptions only grow, and a resolved request takes none. `status` lists them
  with each request, beside its answer (decision, basis, who and when).

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
