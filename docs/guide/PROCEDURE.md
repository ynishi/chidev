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
- The first version of a Project goes through the loop like any other change,
  from right after `init`, so that the record keeps what it is built from: the
  requirements in the requester's words, the interpretations settled on the way
  (Assumptions) and the questions nobody could settle yet (a Crux). What a
  request names has to be there when it opens (see
  [Change requests](../../README.md#change-requests)); the Project is there from
  `init` on and holds a Context, so the part that adds the first Context names
  it (`{"project":true}`). See [The first version](#the-first-version).

## Roles

A role is a way of working with what it may read, not a person; one agent may
take several, one after the other, as long as the Harness role is kept apart.

| role | reads | does |
|---|---|---|
| intake | the request, `status`, `view` | opens the change requests, in the requester's words |
| designer | the requirements and their acceptance examples, the assets (`view`) | puts the Contracts and Interfaces |
| Harness writer (blind) | the Contracts, Interfaces, Harnesses and change requests (`view`); never the code or the requirements | writes the tests and puts the Harness |
| implementer | everything | writes the code, runs the tests, pins it |
| decision maker | the Crux, the Policy | decides a Crux (`cr_decide`), resolves requests |

The Harness writer is blind so that the tests check what the Contracts promise,
not what the code happens to do. A role that has read the implementation does
not put a Harness. What the Harness writer needs from the requirements, an
acceptance example included, the designer has put into a Contract: an example
that shows a promise becomes a Clause (or the wording of one), and an example
that shows something no Clause promises is a question for step 2 (an
Assumption or a Crux) before it is a Case: once the answer gives it a Clause,
a Case checks it like any other. A Harness writer that finds it cannot write a
test from the Contracts and Interfaces sends the gap back to the designer.

Not reading the code keeps the tests from fitting the code; it does not keep them
from sharing a misreading of the requirements with the Contracts. So whoever
writes the Harness, the designer in a fresh session or another agent, is given
only what the record holds
(`{"op":"view","sections":["contracts","interfaces","harnesses","change_requests"]}`:
the Harnesses so that a put keeps the Cases already there, the change requests
for the part a put is `for`), not the requirements, the code or the
conversation that wrote the Contracts.

## The loop

1. **Intake.** Open one change request for the request as it was given: its
   `title`, the `situation` in the requester's words, the `intent`, the
   `issuer`. It is the parent, and it may name no target.
2. **Split.** Read the request against the Contracts and Interfaces (`view`).
   For each part, open a request with `parent` set, naming what it will revise
   or retire (`targets`) and what must stay (`keep`). Name the container when a
   part adds something (the Contract that takes a new Clause). Keep the
   Contracts, the Interfaces and the Harness of one Context in one part unless
   there is a reason to split them. A question that comes up, now or later in
   the design, is settled when it comes up:
   - the text of the requirements, the repository or general knowledge
     settles it to one reading: there is no choice, so record an Assumption,
     with its evidence, and go on;
   - two readings remain: it is a choice, for the decision maker (a Policy
     line can settle the ones that do not matter much). Open a part for it
     then, even in the middle of the design. It names what holds the work its
     answer asks for: the Context, which frames the Clause, the Cases and the
     code of it, whether they are there yet or not. Give it a `crux` with its
     options and a
     recommendation. The change an option makes is its `wording` and its
     `targets`: name there the assets that already exist and that choosing it
     would change beyond what the part names (an Interface whose types it
     changes); a target has to exist when the request opens. Say in its
     `consequence` what else choosing it affects (what a user of the program
     sees, what is not there yet to be named). The work that depends on the
     answer waits: the Clause that promises the point, the Cases that check
     it, and the code that keeps it. The rest goes on. The code written
     meanwhile still does something at that point, but nothing promises it;
     it is not the answer, and the answer's work may change it.

   See [Change requests](../../README.md#change-requests) and
   [Crux, Policy and Assumption](../../README.md#crux-policy-and-assumption).
3. **Design.** Make the change as a `change_set` `for` the part: the Contracts
   and the Interfaces it asks for, in one operation, checked on the state it
   ends in. Keep an Item's id across a rename with `replaces`. Raise a Context
   to `contract` basis when its behaviour needs promises a signature cannot
   carry. See [Change sets](../../README.md#change-sets).
   - Write an Interface's `source` as a caller sees it, since the Harness
     writer has nothing else to call it from: the path a caller writes (the
     crate and module), and for a program, its name, its arguments and what it
     reads and writes. Write what is not a declaration as comments of the
     language (so that the source still parses, as a provider needs when
     `items` is left out), outside the `lines` of the Items: rewording it then
     leaves the Items as they are, though it is still a change of the
     Interface, which moves its revision, makes its Pin stale and, where the
     project measures, its mutation and simulation runs too.
   - An Interface has one Item per declaration a caller uses: for a library,
     each public function or type it calls; for a program, `main`, with its
     subcommands, arguments and what it reads and writes in the comments
     (Clauses promise what they do).
4. **Harness (blind).** The Harness writer writes the tests from the Contracts
   and Interfaces, and puts the Harness in a `change_set` `for` the part whose
   change that put carries: every Case cites the Items it calls and the
   Clauses it checks. Every Clause added or changed is cited by some Case. A
   put replaces the whole Harness, so put it once per part: the put that adds
   the Cases of an answered Crux, and only those, is applied `for` the part
   that holds the Crux. The acceptance examples the requirements give are run
   by Cases too. The designer puts each one into a Clause that names its files
   (for example, "the input `acceptance/a1.stdin` gives the output
   `acceptance/a1.expected` and exit code 0"); the Case reads those files from
   the repository and checks that Clause, so the tests the implementer runs
   before `pin` check the examples as well, and the Harness writer learns of
   them from the Contract alone. A runner outside the Harness checks nothing
   chidev records.
5. **Implement.** Write the code against the Interfaces, run the tests, and
   `pin` each Interface's code once they pass. Where the project measures,
   `measure` the Pin (mutation, simulation) and read what it found; see
   [Measurements](../../README.md#measurements).
6. **Check.** `status` and `check`: every Pin fresh, no stale citation, nothing
   out of a request's frame, no finding left that the work should have
   answered. A stale Pin means implement and pin again; a stale citation means
   bring the tests up to the target, then put the Harness again. An edit (a
   put, a retire, `context_add`, `items_rekey`, a change request opened or
   decided) says only what that edit broke (a citation it made stale, a Pin it
   left behind), so a Clause no Case cites yet, or a Context with no Interface,
   is not said while the Contracts, Interfaces and Harness are still being put:
   it is said here, and by `pin` and `cr_resolve`. To read one Context's share of it,
   `{"op":"check","context":"<name>"}`; `outside` says how many findings
   belong to no Context in particular, so none is lost by narrowing.
7. **Decide.** The decision maker answers each Crux with `cr_decide`: by a line
   of the Policy, or with a reason. Then the work the answer asks for goes
   through 3 to 6.
8. **Resolve.** `cr_resolve` each part with how it ended, why and who says so,
   then the parent. Resolving is a decision; the checks say afterwards where
   the record and the work disagree. A part that does not depend on an open
   Crux may be resolved while the Crux is open: check first that none of its
   Clauses, Interfaces or Cases takes the answer for granted (the work the
   answer asks for is done `for` the part that holds the Crux, which names
   their Context). The part that holds the Crux is not resolved as done until
   it is decided, and the parent waits for all of its parts.

## The first version

The loop runs as above. What is particular is only what the requests name, since
at first nothing is there but the Project.

- The parent holds the requirements in its `situation`, in the requester's words,
  and names no target, so that any work applied to it by mistake is out of its
  frame. It takes no work itself.
- A part names what holds what it adds (see [Change requests](../../README.md#change-requests)):
  the part that adds a Context names the Project, a part that adds to a Context
  names that Context. A frame tells Contexts apart, not two parts of one
  Context. The Project frames everything, so the part that adds a Context does
  only that and is resolved at once; a resolved request takes no more work, and
  the notes its resolve reports about the whole model (a Context with no
  Interface or Harness yet) are expected while it is being drawn.
- An interpretation the text of the requirements, the repository or general
  knowledge settles is an Assumption of the part whose work it settles (the
  parent's, if it settles the request as a whole). Keep the requirements and their acceptance examples as
  files in the repository (a copy of what was given, if it came from elsewhere),
  so that an Assumption can cite them by path; the designer puts what an
  example shows into the Contracts (see [Roles](#roles)).
- A question nothing settles is a part with a Crux and a recommendation. It
  names the Context that holds the work its answer asks for (step 2), and its
  options name what choosing them would change. Once it is decided, the work its
  answer asks for, the Clause and the Cases that cite it, is applied `for` that
  part. The rest of the work, the Interfaces and the Harness too, is applied
  `for` the parts that ask for it (steps 3 to 6).

Each operation goes on one line, as `chidev exec` reads them:

```json
{"op":"init"}
{"op":"cr_open","title":"First version of tally","intent":"change_behavior","situation":"count named items from a list of commands","issuer":"the requester"}
{"op":"cr_open","title":"Add the tally Context","intent":"change_behavior","parent":"First version of tally","targets":[{"project":true}]}
{"op":"change_set","for":"Add the tally Context","puts":[{"put":"context","name":"tally"}]}
{"op":"cr_resolve","title":"Add the tally Context","resolution":"done","reason":"the Context is in","by":{"role":"orchestrator"}}
{"op":"cr_open","title":"Lines and counts of tally","intent":"change_behavior","parent":"First version of tally","targets":[{"context":"tally"}],"assumptions":[{"statement":"a command is + or - and a name, separated by exactly one space","reason":"the requirements give the form, and their example uses one space","evidence":[{"repo":{"path":"REQUIREMENTS.md"}}]}]}
{"op":"change_set","for":"Lines and counts of tally","puts":[{"put":"contract","context":"tally","name":"lines","title":"Lines","clauses":[{"label":"C1","text":"a line is + or - and a name, separated by exactly one space"}]}]}
{"op":"cr_open","title":"What a line that is not UTF-8 is","intent":"change_behavior","parent":"First version of tally","targets":[{"context":"tally"}],"crux":{"kind":"ambiguity","importance":"normal","recommended":{"choice":"not a command","reason":"the counts of the other lines are still printed","confidence":60},"options":[{"choice":"not a command","consequence":"a file with one broken line still gives the counts of the others"},{"choice":"the program stops with exit code 2","consequence":"a file with one broken line gives no counts at all"}]}}
```

The requirements (here `REQUIREMENTS.md`) speak of lines of text and say
nothing of bytes that are not text: whether such a line is one more line that
is not a command, or input the program refuses, is two readings that nothing
settles, so it is a Crux; until it is decided, `status` and `check` say so (`cr.awaiting-decision`),
and only the work that depends on it waits. The Interfaces, the Harness and the
code of "Lines and counts of tally" follow as steps 3 to 6, the Crux is decided
(step 7), then the parts and the parent are resolved (step 8).

## Never

- Add, change or retire a Context or an asset other than as the work
  (`change_set` `for`) of a request: a Context too is added, given its `uses`
  and based on its Contracts in a `change_set`, not with `context_add`,
  `context_uses` or `base_on_contract` on their own. The one edit that has no
  `for`, `items_rekey`, is run while the request it belongs to is open, and its
  resolution says so.
- Put a Harness from a role that has read the implementation.
- Decide a Crux for the decision maker, or answer with a choice the Crux does
  not offer (an own wording is a decision maker's, not an agent's).
- Pin code whose tests do not pass.
- Put a Harness again without changing the tests to make a stale citation go
  away: putting it says the Cases are written against the targets as they are
  now.
- Resolve a request as done when its work is not done; the record is what the
  next reader trusts.
