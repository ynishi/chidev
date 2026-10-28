# 4. A Clause's label is its name, not part of what it promises

Status: accepted

Amended by [ADR 0005](0005-a-new-name-is-a-move-of-the-record.md): a new name is a
move of the footprint; [ADR 0007](0007-the-contract-composes-a-partial-put.md): the
Contract composes a partial put.

Builds on [ADR 0002](0002-every-use-case-that-changes-state-is-a-trigger-and-one-door-keeps.md):
the new put below is an edit with a trigger of its own, and it reaches the
Contract through the one door that already exists. This decision is about what a
Clause's revision counts, and what follows for the operations that change a
Contract.

## Context

A Clause is an entity of its Contract, known by its id. A Case cites it by that
id and the revision it had, and the model's words promise that a citation of it
survives a new label or a reworded sentence. Two things contradicted that.

- **The label sat inside the revised content.** A Clause's content was its label
  and its sentence, and its revision counted changes of the content, so a new
  label alone was a change of the Clause. An Item is the other way round: its key
  stays outside the signature that is revised, and a rename by `replaces` moves
  neither the Item's revision nor the Interface's.
- **The input took the label for the identity.** `contract_put` resolved a label
  to the Clause that has it, or made a new id; it had no `replaces`. A new label
  therefore gave a new Clause, and the old one was dropped, which the rule for a
  dropped Clause that a Case cites refuses unless the Harness is put again in the
  same set. It also offered only a whole replace: to reword one Clause or add one,
  every Clause was sent again, and a slip in one meant to stay changed it with
  nothing to tell it from intended work. A request that added two Clauses to a
  Contract of fourteen and reworded one sent sixteen sentences, thirteen of them
  only to keep them.

## Decision

**A Clause's label is the name a person uses for it, as an Item's key is, and is
not part of what it promises.**

- A Clause has an id, a label, a revision and a sentence. Its revision counts
  changes of the sentence only. `ClauseContent` stays as the pair a caller gives
  (a label and a sentence) to make or change a Contract.
- A Contract's revision moves when its title, the sentence of a Clause, the order
  or the set of Clauses changes. A new label alone moves neither the Clause's
  revision nor the Contract's.
- `replaces` in a Clause of `contract_put`: the Clause that had the old label
  takes the new label and keeps its id, its place and its revision.
- A new operation, `clauses_put` (and the `change_set` entry `clauses`), changes
  some Clauses of a Contract that exists. A Clause given with a label the Contract
  has gets the sentence; one with `replaces` takes the place of the Clause that
  had the old label; any other is appended; `remove` names labels to remove. The
  Clauses not named stay, in their order, and so does the title. The use case
  builds the whole new list from the Contract in the Project and puts it as a
  Contract, so there is no second way into the aggregate; the ids and revisions
  follow from the same rule as a whole put. It is the edit `put_clauses`.
- A refusal says what is wrong with the names, all of it at once: a `replaces` or
  `remove` of a label the Contract does not have (`input.unknown-name`), a label
  that is replaced and still given (`input.replaces-kept`), and a label named
  twice (`input.rename-conflict`).

The stored shape of a Clause (its label and sentence) is kept, and so is the
meaning of every revision already stored: a Clause's revision could only count a
relabel when one happened, and until now no relabel could keep an id.

## Consequences

- A relabel moves nothing. No citation goes stale, no Pin or simulation basis
  goes stale, no change is `cr.out-of-frame`, and a `keep` of the Clause or its
  Contract is not broken by a new name (`cr.keep-moved`).
- A change request whose only work is a relabel has an empty footprint, which
  counts as no work. Resolved as done it gets `cr.resolved-without-work`, and a
  Clause it names as a target reads `not touched yet` (`cr.targets-untouched`
  when the request has other work). The same holds already for an Item renamed by
  `replaces`. Whether a footprint should count a new name as a move, for Clauses
  and Items alike, is a question of its own.
- Names shown in findings, `status` and `view` are the new label at once. A
  Harness put that still cites the old label is refused as an unknown clause. A
  simulation script that prints the old label in the protocol fails on its next
  run, and `sim.stale` does not warn ahead because no revision moved.
- `contract_put` stays the way to make a Contract, to change its title and to
  reorder its Clauses. A partial put does not reorder.
- A label given to two Clauses that the Contract already has is refused as a
  rename conflict, where a whole put used to say the id was given twice.

## Alternatives considered

- Keep the label in the revised content and make `replaces` keep only the id. A
  relabel would then make every citation of the Clause stale, for a change that
  promises nothing new, and it would differ from the Item for no reason.
- Only add `replaces` to the whole put. It keeps the id, but the whole list is
  still sent to change one Clause, which is the cost that was measured.
- Make the partial put a second way to build the aggregate. Two doors would have
  to keep the same rules about ids and revisions; building the whole list and
  using the existing put keeps one.
- Count a new name as a move in the footprint. It would make a rename work that
  frames and keeps judge, and it would have to be decided for Items too; it is
  left as a separate question.
