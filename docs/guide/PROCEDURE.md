# Procedure

The way of working this guide recommends: from a request to code that is pinned
against the assets. The order is a recipe; what is accepted is decided by the
rules of the model, which refuse a change that breaks them whatever the order.
The operations and their fields are in the [README](../../README.md); this file
says when to use which.

## Before the first run

- Put a `chidev.toml` at the project's root (it may be empty) and run from
  anywhere inside it; see
  [The project and `chidev.toml`](../../README.md#the-project-and-chidevtoml).
- Each run of `chidev exec` starts from the store where the last one ended, so
  give a run only the new operations.
- The first version of a Project needs no change request: `init`, add the
  Contexts and put their first assets, then pin. Change requests are for
  assets that exist, since what a request names has to be there when it opens.

## Roles

A role is a way of working with what it may read, not a person; one agent may
take several, one after the other, as long as the Harness role is kept apart.

| role | reads | does |
|---|---|---|
| intake | the request, `status`, `view` | opens the change requests, in the requester's words |
| designer | the assets (`view`) | puts the Contracts and Interfaces |
| Harness writer (blind) | the Contracts and Interfaces only, never the code | writes the tests and puts the Harness |
| implementer | everything | writes the code, runs the tests, pins it |
| decision maker | the Crux, the Policy | decides a Crux (`cr_decide`), resolves requests |

The Harness writer is blind so that the tests check what the Contracts promise,
not what the code happens to do. A role that has read the implementation does
not put a Harness.

## The loop

1. **Intake.** Open one change request for the request as it was given: its
   `title`, the `situation` in the requester's words, the `intent`, the
   `issuer`. It is the parent, and it may name no target.
2. **Split.** Read the request against the Contracts and Interfaces (`view`).
   For each part, open a request with `parent` set, naming what it will revise
   or retire (`targets`) and what must stay (`keep`). Name the container when a
   part adds something (the Contract that takes a new Clause). For a question
   that comes up:
   - the repository and general knowledge answer it: record an Assumption,
     with its evidence, and go on;
   - a choice has to be made: give the part a `crux` with its options and a
     recommendation. The work that depends on the answer waits; the rest goes
     on.

   See [Change requests](../../README.md#change-requests) and
   [Crux, Policy and Assumption](../../README.md#crux-policy-and-assumption).
3. **Design.** Make the change as a `change_set` `for` the part: the Contracts
   and the Interfaces it asks for, in one operation, checked on the state it
   ends in. Keep an Item's id across a rename with `replaces`. Raise a Context
   to `contract` basis when its behaviour needs promises a signature cannot
   carry. See [Change sets](../../README.md#change-sets).
4. **Harness (blind).** The Harness writer writes the tests from the Contracts
   and Interfaces, and puts the Harness in a `change_set` `for` the same part:
   every Case cites the Items it calls and the Clauses it checks. Every Clause
   added or changed is cited by some Case.
5. **Implement.** Write the code against the Interfaces, run the tests, and
   `pin` each Interface's code once they pass. Where the project measures,
   `measure` the Pin (mutation, simulation) and read what it found; see
   [Measurements](../../README.md#measurements).
6. **Check.** `status` and `check`: every Pin fresh, no stale citation, nothing
   out of a request's frame, no finding left that the work should have
   answered. A stale Pin means implement and pin again; a stale citation means
   bring the tests up to the target, then put the Harness again.
7. **Decide.** The decision maker answers each Crux with `cr_decide`: by a line
   of the Policy, or with a reason. Then the work the answer asks for goes
   through 3 to 6.
8. **Resolve.** `cr_resolve` each part with how it ended, why and who says so,
   then the parent. Resolving is a decision; the checks say afterwards where
   the record and the work disagree.

## Never

- Change an asset outside the work of a request, once the Project's first
  version is in.
- Put a Harness from a role that has read the implementation.
- Decide a Crux for the decision maker, or answer with a choice the Crux does
  not offer (an own wording is a decision maker's, not an agent's).
- Pin code whose tests do not pass.
- Put a Harness again without changing the tests to make a stale citation go
  away: putting it says the Cases are written against the targets as they are
  now.
- Resolve a request as done when its work is not done; the record is what the
  next reader trusts.
