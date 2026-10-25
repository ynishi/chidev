# 1. An edit reports what it introduced; a confirmation judges the whole

Status: accepted

## Context

Every put ran every check on the whole Project it left and answered with all
that they found. That is right for a check that says an edit broke something,
and wrong for one that says the model is not finished.

A model is built in steps. A Contract is put before the Interface it is about,
and the Harness whose Cases cite both comes last. Until the Harness is put, every
Clause is cited by no Case, every Item likewise, and the Context has no Harness.
These are true and are the order of the work, not a fault of the edit that
reports them.

The measurement that settled it, on the first Contract of a project: `contract_put`
answered with five `cite.uncited-clause` warnings, one per Clause. The
`interface_put` that followed, which touches no Clause, answered with the same
five again. The next put would have said them a third time. A reader of the
answers learns to skip the findings, and a finding that means something (a
citation this edit made stale) is among the ones skipped.

A second effect is on refusal. A mandatory check refuses the use case that
finds something at `error`. Judged on the whole Project after an edit, it refuses
an edit for a finding the Project already had, which the edit neither made nor
can mend.

## Decision

A hook is held to one of two things, by whether the use case edits or confirms.

- **An edit (`put`) reports what it introduced.** The checks of the put hook run
  on the Project before the edit and on the Project after it. A finding of the
  state after is introduced when the state before has no finding of the same
  rule about the same subject, or has one that stands on fewer grounds. The edit
  answers with the introduced findings, and a mandatory check refuses it only
  for those. The record of the run still counts everything the checks found in
  the state after.
- **A confirmation (`pin`, `cr_resolve`) and `check` judge the whole Project.**
  What is wrong with the model as it stands is said where something is confirmed
  or asked for.

Each check says which it is, in its descriptor (`kind`): a **change** is a check
that says an edit made something stale or out of place; a **state** is a check
that says something is not there, not settled or not measured. A change runs at
`put` and at the confirmations; a state runs at the confirmations and on demand,
never at `put`. A test holds every builtin to it, so that a check cannot gain a
`put` hook while declaring itself a state.

Two runs of a check find "the same thing" when the finding's rule and subject are
the same; that identity is what a change request already points at. The reason
a finding stands is carried apart, as typed **grounds**: the target a citation
cites and the revision it has now, the thing a Pin stands on that moved and its
revision, the target a request keeps and its revision, the change set that moved
a target outside the frame of its request, the policy line a decision rests on
and whether it is gone or reworded (by the line's own revision, not the Policy's).
A ground is the mark of a fact. The message is never compared, since a message
that counts things changes without the edit having made anything worse. A put that
leaves every ground as it was has introduced nothing, and one that adds a ground
or moves the mark of one has introduced that part of it.

External checks cannot give grounds: the protocol has no typed reason, so the
message is the only mark of what makes a finding hold. At `put` they are compared
by subject and message against the saved result of the same rule (by subject
alone, every finding about the whole Project would be one), and only
when that result describes the model before the edit (the same input and the
same registration). With nothing to compare against, every finding counts. A run
that failed refuses a gate whatever was saved.

`check` also takes a Context, which narrows the findings to those about it and
counts the rest as `outside`, so that a person who is working in one Context can
read the whole model's findings for it without the rest.

## Consequences

- A put answers with little, and what it says is the edit's own.
- A model built in the usual order is not told, at each step, that it is not
  built yet. It is told so by `check`, `status`, and by `pin` and `cr_resolve`,
  where the model is claimed to be in a state.
- A finding a put introduced and the person ignores is not said again by the
  next put. It is still said by `check`.
- `CheckView.before` was read by no check and is removed; `CheckView.after` is
  `project`, since a view is one state. A change of two states is `Change`. This
  is a change of the public API of `chidev-core`.
- A put runs the builtin checks twice. They are pure functions of the model.
- The checks that say the model is not finished or not settled lost their put
  hook, and `check_list` shows the `kind` of each check.
- Known limitation: an external finding that gains a new reason about the same
  subject is not new, because the protocol has no typed reason to compare. And
  when no saved result describes the model before the edit, a finding the Project
  already had can refuse a put for a gate. That is the fail-closed side.

## Alternatives considered

- Only move the state checks off the put hook. The change checks (a stale
  citation, a stale Pin) would then be said again by every put that has nothing
  to do with them, and an external gate would keep refusing puts for findings the
  Project already had.
- Remove the put hook. A stale citation or a stale Pin is exactly what an edit
  should be told at the moment it makes it.
- Hooks per kind of put. The line that matters is edit against confirmation,
  not which asset is put; more hook words would reach the policy and the
  external registration for no gain.
- Ask the checks to write, in the message, what is new. A message is wording, and
  comparing wording makes a rewording of it a new finding.
