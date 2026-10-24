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

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
