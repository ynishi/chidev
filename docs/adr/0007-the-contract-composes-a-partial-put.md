# 7. The Contract composes a partial put

Status: accepted

Amends [ADR 0004](0004-a-clause-label-is-its-name-not-part-of-what-it-promises.md)
on one point: who builds the list of Clauses of a partial put. Builds on
[ADR 0002](0002-every-use-case-that-changes-state-is-a-trigger-and-one-door-keeps.md):
the one way into the aggregate stays one.

## Context

ADR 0004 added a partial put of a Contract (`clauses_put`, and the `clauses` entry
of a change set): some Clauses are set, some removed, the rest stay. It had the use
case build the whole new list of Clauses from the Contract in the Project and put it
as a Contract, so there was no second way into the aggregate. Two things followed
that the aggregate should not have left to the use case.

- **The change set had no partial put.** The domain's change set knew a Contract put
  only as the whole list, so the use case composed a `clauses` entry into a whole
  Contract. It cannot do that for a Contract the same set makes (the Project does not
  have it yet), so for that case it built the refusal itself, a Contract named twice,
  which is a rule of the change set and was written in the use case. The entry was
  never shown to the domain.
- **The rules about names lived outside the aggregate.** Which label a Clause keeps its
  id by, what `replaces` may name, when a label is named twice or against itself, and
  the order Clauses come in, are rules about the content of a Contract. They were in a
  function of the use case, next to the code that draws ids and reads input.

## Decision

**The Contract composes the partial put, and the change set carries it.**

- `AssetPut::Clauses { context, id, edits, remove }` is the partial put in the change
  set. It is applied with the Contracts and Interfaces. It is a put of the Contract,
  so `repeats` names it under the same key as a Contract put: a `contract` entry and a
  `clauses` entry for one Contract, or two `clauses` entries, are a Contract named
  twice (`change-set.named-twice`), refused by the domain.
- `ClauseEdit` is a Clause as written: a label, a sentence, the label it `replaces`,
  and the id it takes when it makes a new Clause. The caller draws that id: the domain
  draws no ids. An edit that sets a Clause the Contract has keeps that Clause's id and
  does not use the one it carries.
- `Contract::clauses_whole(old, edits)` and `Contract::clauses_partial(&self, edits,
  remove)` hold the rules about names, as two pure functions with no mode flag over one
  check. They give the list of Clauses with their ids, or every problem with the names
  at once (`ClauseNameProblem`: an unknown label, a label named twice, a label replaced
  and still given). A label the Contract does not have, given twice, is not a problem of
  names: the Contract put refuses it when it is put (`DuplicateLabel`).
- The one way into the aggregate stays one. For a `Clauses` put the Context takes the
  Contract it has, has it compose the list with `clauses_partial`, and puts that list
  with the Contract's own title through the Contract put it already has
  (`Context::put_contract`). Problems with the names there are refused all at once,
  each as a `ContractError::ClauseName`, which is a violation of the rule the use case
  already gives for it (`input.unknown-name`, `input.rename-conflict`,
  `input.replaces-kept`); it is reached only by a caller that did not check the names
  first. A Contract the Context does not have is `NoSuchAsset`.
- The use case reads the input into edits (drawing one id per edit), calls
  `clauses_whole` for a `contract` entry and `clauses_partial` for a `clauses` entry to
  get the label to id map that the Cases of the set cite, and maps the problems to the
  refusals it already had (`input.unknown-name`, `input.rename-conflict`,
  `input.replaces-kept`); their codes and messages do not change. A `contract` entry
  still goes to the domain as a Contract put. For a `clauses` entry whose Contract the
  set makes, it resolves nothing and sends the entry on, and the domain refuses the
  repeat.

## Consequences

- The use case names no repeat any more: the refusal of a Contract named twice comes
  from `Project::apply` for a `clauses` entry as for any other asset. A set that also
  has problems the use case finds while it plans (an unknown name) says the repeats
  with them: the use case asks `Project::repeated` of what it planned, so the rule
  stays in the domain and every problem is said at once.
- The rules about names are tested on the Contract, without a store or a use case.
- One id is drawn for each edit, whether or not it makes a Clause, so ids that
  come after it in the same use case are drawn later than before. An id is a
  time-ordered UUID in the store, and one that goes unused is harmless.

## Alternatives considered

- Keep the composition in the use case. It works, but the change set cannot see a
  `clauses` entry for a Contract it makes, so the use case keeps naming that repeat
  itself, and the rules about names stay outside the aggregate.
- A second door into the aggregate: a `Context` method that revises some Clauses of a
  Contract in place, beside the Contract put. Two doors would have to keep the same
  rules about ids, revisions and `DuplicateLabel`; composing the list and using the
  existing put keeps one.
