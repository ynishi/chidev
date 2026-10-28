# 5. A new name is a move of the record

Status: accepted

Builds on [ADR 0004](0004-a-clause-label-is-its-name-not-part-of-what-it-promises.md):
a Clause's label and an Item's key are names, and a new name moves no revision.
This decision is about what the footprint of a change set, the record kept for the
change request it was applied for, makes of a new name.

## Context

The footprint was the set of targets whose revision differs between the Project
before a change set and the one it leaves. ADR 0004 made a new name a change that
moves no revision, and left open whether a footprint should count a new name as a
move, for Clauses and Items alike. With the footprint as it was, a new name left no
trace:

- A change set `for` a request whose only work was a new name recorded an empty
  footprint, which the request drops as no work. Resolved as done, the request was
  `cr.resolved-without-work`.
- A Clause or Item the request named as a target read `not touched yet`, though it
  had been given the new name the request asked for.
- Nothing in `status` said that a name had changed, or what it had been.

The record of a request is about what a person changed. Revision is the measure for
what is promised (citations, Pins, keeps); a name is a change that is deliberately
not a revision change, and the footprint had one kind of move where the model has
two.

## Decision

**A new name is a move of the Clause or Item in the footprint, though it moves no
revision.**

- A move carries what was done to its target: its revision moved (made, revised or
  gone), it was given a new name, or both in one change set. A new name is the name
  it had and the name it has: a label for a Clause, a key for an Item. A move of a
  Clause cannot carry a key, nor a move of an Item a label.
- A Clause or Item present before and after a change set with the same id and
  another label or key is given a new name. When its revision moved too, it is one
  move with both. A Clause removed while another is added under its label is two
  ids, so it is no new name.
- A new name moves no container: the Contract or Interface holding the Clause or
  Item moved no revision, so it is not in the footprint for it. The name-only move
  keeps the containers the target is in, as every move does.
- Work, frame and progress read every move.
  - A request whose only work is a new name has work: resolved as done it is not
    `cr.resolved-without-work`.
  - A target with a move in it or on it reads `revised`; there is no progress word
    of its own, and `work` in `status` has the detail.
  - A new name outside the frame of the request it was applied for is
    `cr.out-of-frame`, for the Clause or Item alone.
- A `keep` is judged by revision, so a new name breaks none: the name is not part of
  what a keep protects (ADR 0004). No citation goes stale for it either.
- `status` lists, for each change set of a request, the targets whose revision it
  moved as `moved` and the Clauses and Items it gave a new name as `renamed`, each
  with the target, the name it had and the name it has.
- Storage: a stored move may carry the name it had and the name it has, and whether
  its revision moved with it. A move stored without it is a revision move, which is
  what every move stored before was; no stored document changes and the schema is
  not bumped. A name that cannot be read, or one on a target that is neither a Clause
  nor an Item, makes the document unreadable.

## Consequences

- A rename-only request is work and its target reads `revised`; the finding that it
  had no work no longer fires for it, and `cr.targets-untouched` no longer names a
  target that was renamed.
- A frame that names the Contract holds the Clauses in it, so a relabel there is
  inside; one that names other Clauses only has the relabel outside, and says so.
- A name that moves nothing else is told apart from a revision in `status`, so a
  person reading the work of a request can see that a label changed and what it was.
- An Item's key and a Clause's label are the only names recorded. A Case is found by
  its name and an asset is named in the store, not in the Project, so they have no
  rename to record.
- A footprint stored before has no names. Reading it as revision moves is true to
  what it recorded.

## Alternatives considered

- Count a new name as a revision. It makes every citation of the Clause or Item
  stale and breaks every `keep` of it, for a change that promises nothing new, and it
  contradicts ADR 0004.
- Keep a rename log beside the footprint. A change set would be recorded in two
  places, and work, frame and progress would have to read both to say what one set
  did.
- Give a rename a progress word of its own. `revised` already means that a footprint
  moved the target or something in it and it is still there, and `work` has the
  detail; a new word would make a reader choose between two for one request.
