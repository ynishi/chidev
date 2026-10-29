# 2. Every use case that changes state is a trigger, and one door keeps it

Status: accepted

Amended by [ADR 0008](0008-a-forced-correction-is-a-trigger-of-its-own-kind.md): a
forced correction is a fifth kind of trigger.

Builds on [ADR 0001](0001-edit-reports-introduced-confirm-judges-whole.md), which
decided what an edit and a confirmation are told. This decision is about which
use cases are held to that, and how the code makes sure none is left out.

## Context

ADR 0001 gave the checks a moment to run at: `put` for an edit, `pin` and
`resolve` for the confirmations, `check` for a question. The code held four use
cases to a moment: the puts and retires (through `apply_change_set`), the Policy,
`pin` and `cr_resolve`. Every other use case that changed what the store keeps
wrote to it directly, so no check ran on it, and neither did an external check
registered as a gate.

The measurement that settled it was a count of the writes in the use cases. Of
the operations that change state, eight edit the model or the record of a change
request and kept it without a check: `context_add`, `context_uses`,
`base_on_contract`, `items_rekey`, `cr_open`, `cr_decide`, `cr_assume` and
`cr_reopen`. What that cost:

- `items_rekey` moves the revision of an Item whose lines declare something else,
  which makes every Case that cites it stale (`cite.stale`). It said nothing.
- `context_add`, `context_uses` and `base_on_contract` edit the model the same
  way a `change_set` entry of the same name does, and that entry was checked; so
  the same edit was checked or not by the way it was written.
- An external check registered as a gate, at `put`, was never asked about any of
  the eight, so a gate that was meant to refuse an edit was walked around by one.

The leak was not one forgotten call. A use case reached the store by a method any
of them could call (`Store::commit` with a bag of changes), and what was checked
and what was kept were assembled apart: the state the checks ran on was built
from one set of variables and the changes to keep from another.

A second confusion sat in the name. `Hook` held four values of different kinds:
a family of operations (`put`), two single operations (`pin`, `resolve`) and a
question (`check`). "Put a hook on `context_add`" was the wrong picture: the
operation is the trigger, and when its checks run is a property of its kind.

## Decision

**A use case that changes state is a trigger, of one of four kinds, and the kind
decides its moment.**

- An **edit** changes what the model says. It is held to the moment `edit`: the
  checks run on the model before it and after it, and it is told, and refused by
  a mandatory check or an external gate, only for what it introduced (ADR 0001).
- A **confirmation** says the model is in a state (`pin`, `cr_resolve`). It is held
  to the moment of what it confirms and judges the whole model.
- A **question** (`check_run`) is held to the moment `check`. It keeps what the
  checks said, external programs included, and never refuses: it changes no asset.
- A **record** keeps a fact (`init`, `measure`, `simulate`, `finding_report`) and
  runs no check. It cannot change the model: its type has no Project, Pin, change
  request or name in it. Only `init` makes a Project, through an entry of its own.

The moment is read from the type of the trigger. No table pairs a use case with a
moment, and the list of triggers is written once, in `app::commit`, where each
kind lists its use cases.

**The record of a change request is edited like the model is.** `cr_open`,
`cr_decide`, `cr_assume` and `cr_reopen` are edits: they run the checks of an
edit and are seen by an external gate. ADR 0001 wrote of an edit as an edit of the
model; a change request is part of what a check reads (`cr.keep-moved`,
`cr.out-of-frame`), so an edit of it can introduce a finding, and that is said.

**One door keeps.** The store keeps only a `Gated`: a change together with the
trigger that made it, which only the entries of `app::commit` can build. There is
one entry for each kind of trigger (and one for `init`), and each runs what its
kind asks for before it builds the `Gated`. An edit and a confirmation hand the
entry a `ModelDelta`, the part of the model they change, keyed by what the store
keys its rows by; the entry lays the delta on the model it read to get the state
to check, and keeps the delta, so what is checked is what is kept. A use case
that is added without an entry cannot keep anything, and one that skips its checks
cannot be written without the one function that builds a `Gated`.

**The checks a use case ran are kept with its name.** A row of the record of runs
has the trigger beside the moment. The word of a trigger is stored, so it is
never changed once written.

**The words match.** What a check says it runs at is `moments` (it was `hooks`),
and the moment of an edit is `edit` (it was `put`), in `chidev.toml`, in judged
rule files, in `check_list` and in the argument of `check_rules` (`moment`). The old
key and word are refused with the new one named, not ignored. The request an
external program is sent is version 2 (`moment`, not `hook`); a reply of version 1
or 2 is accepted. The request names the moment and not the trigger, so the names
of the use cases are not a protocol a program can depend on.

**A rename does not stale what is saved.** The digest of an external registration,
and the stamp of a saved result, are made of the words the first version used
(`hooks`, `put`) through a copy written for the digest, so a result saved before
the rename is still current after it. The digest form is fixed at the first
version's; the word `put` stays what an edit is counted as in a digest.

## Consequences

- Eight operations now answer with `findings` and can be refused by a mandatory
  check or an external gate for what they introduce. A gate registered at `edit`
  is asked about them.
- `items_rekey` says which citations it made stale, when it makes any.
- `init` runs no check, and neither do `measure`, `simulate` and `finding_report`;
  the model is not changed by them, or (for `init`) there is no model before.
- The public functions of `chidev-core` that returned nothing or the value alone
  now return it with its findings (`Checked`), as the puts do: this is a change of
  its API. `Store::commit` takes a `Gated`.
- A database made before the trigger was kept reads each of its rows with no
  trigger. A row whose moment was stored as `put` is read as an edit.
- A file written with the old key or word stops being read, with the new word in
  the message. This is a change for people who wrote `hooks` or `put`.

## Alternatives considered

- Run the checks of an edit at the eight use cases, one by one. This closes what
  was counted and nothing after it: the next use case would again have to remember
  to ask.
- Keep `Hook` and add a value for each use case. It ties the words in a user's
  configuration to the names of use cases, and it puts the question of when a
  check runs in the same list as which operation it is.
- Keep the old words outwardly and rename only in the code. A configuration then
  says `hooks` and `put` for something the code calls a moment and an edit, which
  is the confusion this decision removes.
- Make the digest from the new words. Every saved result of an external check
  would be stale at once, and a gate registered at `edit` alone would refuse every
  edit for findings it already had, with nothing saved to compare with, since a
  refused edit saves nothing.
- Pass the trigger to an external program. It would fix the names of the use
  cases as a protocol; the moment is what a program needs, and ADR 0001 gave the
  same reason against a word for each kind of put.
