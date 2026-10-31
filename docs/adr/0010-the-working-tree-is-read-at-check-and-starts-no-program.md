# 10. The working tree is read at `check` alone, and reading it starts no program

Status: accepted

## Context

The code is a cache pinned to the assets (see the README). A Pin records the digest of
the code it accepted, and the store keeps that text beside the Pin; the source of a
Harness is in the model. The project's `[[measure.layout]]` says which file of the
working tree holds each of them. Only `measure` uses the layout, and it writes the
model's text over a copy of the project: it never reads the tree's own file.

So the text of a placed asset is held twice, and nothing in the model records whether
the two are the same. The observed consequence: a signature changed in a pinned file,
on disk, leaves `check` with no finding. Only after the Interface is put again do
`cite.stale` and `pin.stale` appear, because they compare revisions of the model. A
project in which the code is edited in the tree, as most are, learns nothing from
`check` about code that left its Pin until somebody puts it again.

Two things are missing from the model, not from a tool:

- **The copy is not a value.** The domain has no path, rightly; but it also has no
  value for "what the file holds now", so no rule can say the file left the text.
- **Where the observation belongs.** A copy is a fact about the outside world at the
  moment of looking, as the settings of a runner are. It is not something a Pin, an
  edit or a resolution claims.

A constraint binds the answer: `status` and `check` start no external program (README,
"External checks"); `check_run` is where programs run.

## Options

| option | what it does | rating |
|---|---|---|
| A. An external check | A program registered at the `check` moment compares the files with the model | 4/10: it cannot see the Pin's digest; `check` and `status` do not start it, so they show a saved result as `current` while the file has moved |
| **B. Copies as an input of the `check` moment** | A port reads the files as values of the domain; rules of the domain compare them with what the model holds | **8/10**: the same shape as the settings a measurement is compared with; no program |
| C. `chidev sync` | The tree is read and put again | 5/10: the tree would overwrite the assets, the opposite of the model being the authority; a new file is not seen |
| D. Only the digests | B without anything per Item | 6/10: the cheapest B; it is B's first step |

## Decision

**B**, in steps. The first compares digests.

**A copy is a value.** What a port reads of the file that holds one asset is a
`CopyRead` (unplaced, missing, unreadable, or the digest of its bytes). It is set
against what the model holds by one function for the code of an Interface (against its
Pin: `same`, `moved`, `unpinned`, `missing`, `unreadable`, `unplaced`) and one for the
source of a Harness. The copies of a Project are a `TreeNow`; an asset it holds no copy
of is unplaced, and the one function that says so is the one that reads it. A digest
is the digest of the file's bytes after they are known to be UTF-8, which is how the
text of a Pin is made, so the two are equal exactly when the texts are. Nothing is
normalised: a changed line ending is a change.

**Which moment.** `check` alone: the rules `tree.code-moved`, `tree.harness-moved` and
`tree.unread` are checks of the state of the world, and none runs at an edit, a `pin`
or a `cr_resolve`.

- Not at an edit ([ADR 0001](0001-edit-reports-introduced-confirm-judges-whole.md)):
  an edit does not change the tree, so it introduces nothing there to report.
- Not at a confirmation. What a `pin` claims is that the code it was given is accepted
  for this revision, and what a `cr_resolve` claims is that the work of the request is
  done in the model. A copy is not what either claims. The runs of examples are given
  to a confirmation for a reason that does not apply: a confirmation must not claim a
  state the latest run contradicts
  ([ADR 0009](0009-environments-are-assets-and-examples-are-a-kind-of-harness.md)),
  and the copy contradicts no run.
- Not at a confirmation for cost as well. A confirmation already judges the whole
  model, and reading every placed file at every one would add the size of the tree to
  each.
- At `check` because a rule about what is outside the record belongs where
  `mutation.stale` is: it reads the settings now and runs at `check` only.

**One place reads the inputs of the moment.** `check` and `status` build their view in
one use case, and the question `check_run` builds its own. A reading added to one of
them is a reading the others do not have, so what the moment reads besides the model is
collected by one function that all three call. The copies are read there, once per
call, by the port, for every asset together.

**The process promise stands.** The port reads files and starts nothing, so `status`
and `check` still start no program. A comparison per Item, which needs the provider of
the Interface's language to find the Items, would start one; it would be an exception to
the promise and would have to be opt-in and written into the README where the promise is
made. It is not part of this decision.

**One filter for the code of a Pin.** The code a Pin accepted is the text the store
kept if its digest is the Pin's. Two use cases held that filter each; it is one
function now, so a use case that reads the accepted code does not copy it.

## Consequences

- `check` now says when the file of a pinned Interface is not the code its Pin accepted.
  A project that keeps pinned code apart from its tree on purpose turns the family off
  (`[checks."tree."] level = "off"`); `status.tree` is then `null`, for nothing was
  looked at.
- The comparison is by file. Two Interfaces laid out in one file move together, and a
  finding does not say which Item changed.
- A Harness is not a subject of a finding, so a finding about the copy of a Harness is
  about its first Case, as the findings of examples and simulations are.
- An asset that no layout places has no copy; it is counted in `status.tree.unplaced`
  and is the subject of no finding. An Interface with a file and no Pin is compared with
  nothing: `pin.unpinned` says so, and `status.tree.unpinned` lists it.
- The copy is an observation at the moment of looking. A saved result of an external
  check is not affected, and its freshness is still the comparison of the model sections
  it was given.
- `CheckView` gained a field, `tree`: a change of the public API of `chidev-core`.

## Steps

Each is one change, verifiable on its own:

1. The digests, the three rules and `status.tree`, with no program started.
2. The Items of a moved copy, opt-in, with the provider.
3. Files that match a pattern and that no layout places.
