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
| decision maker | the Crux, the Policy | decides a Crux (`cr_crux_decide`), resolves requests |

The decision maker is a role that applies the Policy, an answering agent, or a
person. The authority to decide is managed outside chidev: chidev keeps `by` as
the decision maker declared it and checks nothing against it (see
[Crux, Policy and Assumption](../../README.md#crux-policy-and-assumption)).

The Harness writer is blind so that the tests check what the Contracts promise,
not what the code happens to do. A role that has read the implementation does
not put a Harness. What the Harness writer needs from the requirements, the
promise of an acceptance example included, the designer has put into a Contract
(the example's files themselves are given as they are, for the Harness of
`examples`): an example that shows a promise becomes a Clause (or the wording
of one), and an example
that shows something no Clause promises is a question for step 2 (an
Assumption or a Crux) before it is a Case: once the answer gives it a Clause,
a Case checks it like any other. A Harness writer that finds it cannot write a
test from the Contracts and Interfaces sends the gap back to the designer, who
treats it as a question of step 2: a Clause where the requirements settle it,
an Assumption where an interpretation does, a Crux where two readings remain,
or a comment of the Interface reworded where it says more than any Clause
promises. The answer is work `for` the part the question was raised in, and the Harness
writer then puts the Cases that check it. A resolved part takes no work:
`cr_reopen` it first, or open a new request. Reopening leaves a resolved parent
resolved, so reopen a part while its parent is still open.

Not reading the code keeps the tests from fitting the code; it does not keep them
from sharing a misreading of the requirements with the Contracts. So whoever
writes the Harness, the designer in a fresh session or another agent, is given
only what the record holds
(`{"op":"view","sections":["contracts","interfaces","harnesses","change_requests","cruxes"]}`:
the Harnesses so that a put keeps the Cases already there, the change requests
for the part a put is `for`, the Cruxes for the questions a part waits on), not
the requirements, the code or the conversation that wrote the Contracts. A request names its targets by id
(`expects`, each with its `id`), and the same `view` gives each Context and
asset its id beside its name, and each request its `parent`: the part a put is
`for` is the one whose parent is the request and whose target holds the change.
The `cruxes` give each question with the id of the request it stops
(`raised_in`), its options and, once decided, the decision: a part that some
Crux names there waits on that question. `view` does not give a request's
`situation` or Assumptions.

## The loop

1. **Intake.** Open one change request for the request as it was given: its
   `title`, the `situation` in the requester's words, the `intent`, the
   `issuer`. It is the parent, and it may name no target.
2. **Split.** Read the request against the Contracts and Interfaces (`view`).
   For each part, open a request with `parent` set, naming what it will revise
   or retire (`targets`) and what must stay (`keep`). Name the container when a
   part adds something (the Contract that takes a new Clause). Keep the
   Contracts, the Interfaces and the Harness of one Context in one part unless
   there is a reason to split them; parts that share a Harness can then be
   taken through steps 3 and 4 side by side (see step 4). Keep the unit to
   protect: an Item, not the whole Interface, when only its signature must stay. Do not keep what another
   part targets: a part that keeps an Interface and a part that names one of its
   Items contradict each other. `cr.keep-targeted` in an answer says so; resolve
   the wrong part as `{"superseded":"<new title>"}` and open it again with the
   right `keep`, before any work. A question that comes up, now or later in
   the design, is settled when it comes up:
   - the text of the requirements, the repository or general knowledge
     settles it to one reading: there is no choice, so record an Assumption,
     with its evidence, and go on;
   - two readings remain: it is a choice, for the decision maker (a Policy
     line can settle the ones that do not matter much). Raise it with
     `cr_crux_raise` in the request it came up in, even in the middle of the
     design; it does not get a part of its own. Give it its `kind`, its
     `question`, its `options` and a recommendation. The change an option makes
     is its `wording` and its `targets`: name there the assets that already
     exist and that choosing it would change (an Interface whose types it
     changes); a target has to exist when the Crux is raised. Say in its
     `consequence` what else choosing it affects (what a user of the program
     sees, what is not there yet to be named). A target inside what the request
     names becomes the request's work when the option is chosen. A target
     outside it never does: the request's scope is what it was opened with. So
     an answer that reaches outside is decided with `beyond`: either a new
     request opened for what it adds, naming those targets, and taken next as
     a request of its own, not as another part under the same parent
     (`{"requests":["<title>"]}`), or a stated decline (`{"declined":"<why>"}`).
     Deciding it without either is refused (`cr.crux-answer-outside`). An
     answer that shows the request itself was wrong is not a reason to widen
     it: resolve it as `{"superseded":"<new title>"}` and open the corrected
     one. The request
     waits: it is not resolved, however it would end, until its Cruxes are
     decided. The work that depends on the answer waits as well: the Clause that
     promises the point, the Cases that check it, and the code that keeps it.
     The rest goes on, in the other parts; work that should not wait for the
     answer belongs in a request of its own. The code written meanwhile still
     does something at that point, but nothing promises it; it is not the
     answer, and the answer's work may change it.

   See [Change requests](../../README.md#change-requests) and
   [Crux, Policy and Assumption](../../README.md#crux-policy-and-assumption).
3. **Design.** Make the change as a `change_set` `for` the part: the Contracts
   and the Interfaces it asks for, in one operation, checked on the state it
   ends in. When a Crux raised in the part is decided, the targets of the
   option chosen that are inside what the part names become expectations of
   the part, so the work the answer asks for there is `for` the part too. What
   the answer changes outside the part is the work of the request its `beyond`
   names, `for` that request (see step 2). A resolved part takes no work:
   reopen it, or open a new request. Keep an Item's id across a rename with `replaces`, and a Clause's id
   across a new label the same way. A new name is work of the part it is `for`
   (`status` shows it under `renamed`), so give it in the set of the part that
   names the Clause or Item, or its Contract or Interface: anywhere else it is
   `cr.out-of-frame`. To change some Clauses of a Contract (reword
   one, add one, remove one) put only those with `clauses_put`, or the `clauses`
   entry of the set: the Clauses not named stay as they are, so a Clause
   meant to stay cannot change by a slip. Use `contract_put` to make a Contract,
   to change its title or to put its Clauses in another order. Raise a Context to
   `contract` basis when its behaviour needs promises a signature cannot carry.
   See [Change sets](../../README.md#change-sets).
   - Write an Interface's `source` as a caller sees it, since the Harness
     writer has nothing else to call it from: the path a caller writes (the
     crate and module), and for a program, its name, its arguments and what it
     reads and writes. Write what is not a declaration as comments of the
     language (so that the source still parses, as a provider needs when
     `items` is left out), outside the `lines` of the Items: rewording it then
     leaves the Items as they are, though it is still a change of the
     Interface, which moves its revision, makes its Pin stale and, where the
     project measures, its mutation and simulation runs too. A comment that
     gains or loses lines moves the declarations after it: count their `lines`
     again, or keep such comments after the last Item, or leave `items` out
     where a provider finds them.
   - An Interface has one Item per declaration a caller uses: for a library,
     each public function or type it calls; for a program, `main`, with its
     subcommands, arguments and what it reads and writes in the comments
     (Clauses promise what they do).
4. **Harness (blind).** The Harness writer writes the tests from the Contracts
   and Interfaces, and puts the Harness in a `change_set` `for` the part whose
   change that put carries: every Case cites the Items it calls and the
   Clauses it checks. It reads `cruxes` in `view` to see which part a question
   stops and, once decided, what was decided. Every Clause added or changed is
   cited by some Case. A put replaces the whole Harness and is applied `for`
   one part: the put that adds the Cases of an answered Crux, and only those, is
   applied `for` the part the Crux was raised in. The Cases that follow a
   part's Clauses are that part's work, so they are inside its frame. When the
   Cases of one Harness follow the Clauses or Items of several parts, the parts
   can be designed side by side: a put `for` one part brings a Case's citation
   up to the revision of what it cites only where that part's frame holds it,
   and leaves the Cases another part's design made stale as they are, stale and
   unmoved, until the Harness is put `for` that part. A Case that cites the work
   of two parts is brought up on each part's citations by that part's put, and
   moves in each. A Case whose code a put changes moves whichever part the put
   is `for`, so a change to a Case's code goes in the put `for` the part it is
   for. The Harness is not put `for` the
   parent: its frame is its own targets, not its parts' (see
   [Attribution and frame](../../README.md#attribution-and-frame)). A Case
   the implementer finds wrong against its Clause goes back to the Harness
   writer with that Clause,
   and the writer puts the corrected Harness `for` the same part. The acceptance examples the
   requirements give are a Harness of their own, of `kind` `examples` (see
   [Acceptance examples as a
   Harness](../../README.md#acceptance-examples-as-a-harness)). The designer puts
   the promise each example shows into a Clause (in words of the promise; the
   Clause does not need to name the files), and the Harness writer puts the
   examples as they are given, with `examples_dir`, each Case citing the Clause
   its example shows. The model holds the examples, so a request that changes
   one (a new line of an expected output) is a put of that Harness `for` the
   request: the example's Case and the Harness go to their next revision and the
   request has touched them; reword the Clause only when the promise itself
   changes. The examples are run by `chidev accept --external`, registered as an
   external check that gates `pin` and `resolve` (see [Acceptance
   examples](../../README.md#acceptance-examples-chidev-accept)); it reports
   each failing example about the Clauses its Case cites.
5. **Implement.** Write the code against the Interfaces, run the tests, and
   `pin` each Interface's code once they pass. chidev does not run the tests
   itself; to have it refuse a `pin` while they fail, register a program that
   runs them as an external check at the `pin` moment with `gate = true` and a
   `reads` of at least one section (see
   [External checks](../../README.md#external-checks)). The program runs the
   tests of the working tree, not the code the `pin` gives, so the gate holds
   when that code is the tree's: pin it with `code_file`. Check the gate once
   before relying on it: `pin` the code while the new tests still fail, and see
   it refused (`"ok":false`, the external check at level `error` with the failing
   tests in its evidence); then write the code and pin again. Where the project
   measures, `measure` the Pin (mutation, simulation) and read what it found;
   see [Measurements](../../README.md#measurements).
6. **Check.** `status` and `check`: every Pin fresh, no stale citation, nothing
   out of a request's frame, no finding left that the work should have answered.
   A project that does not measure keeps the note `mutation.unmeasured` for each
   pinned Interface: it says no run was made, not that the work missed
   something. A stale Pin means implement and pin again; a stale citation means
   bring the tests up to the target, then put the Harness again `for` the part
   whose frame holds the citation (a put `for` another part leaves it stale). An edit (a put,
   a retire, `context_add`, `items_rekey`, a change request opened, a Crux
   raised or decided) says only what that edit broke (a citation it made stale, a Pin it left
   behind), so a Clause no Case cites yet, or a Context with no Interface, is
   not said while the Contracts, Interfaces and Harness are still being put: it
   is said here, and by `pin` and `cr_resolve`. To read one Context's share of
   it, `{"op":"check","context":"<name>"}`; `outside` says how many findings
   belong to no Context in particular, so none is lost by narrowing.
7. **Decide.** The decision maker (a role that applies the Policy, an answering
   agent, or a person) decides each Crux with `cr_crux_decide`: by a line of
   the Policy, or with a reason. Deciding that the behaviour is left
   unspecified, or withdrawing a question that is not needed any more, is a
   decision too; a Crux is not left without one. (A question about the words of
   the model cannot be left unspecified.) An answer whose option names a
   target outside the part is decided with its `beyond` (step 2). Then the
   work the answer asks for goes through 3 to 6, `for` the part the Crux was
   raised in, and what it adds outside, `for` the request `beyond` names. A
   decision is replaced once: a second Crux that supersedes the same decision
   is refused (`cr.supersedes-superseded`); supersede the one that replaced it.
8. **Resolve.** `cr_resolve` each part with how it ended, why and who says so,
   then the parent. The decision maker resolves (see Roles), on the
   implementer's report that the work is done and the checks of step 6 are
   clean; `by` names the decision maker, as for a decision (`{"role":..}` or
   `{"person":..}`), not the role that did the work. Resolving is a decision;
   the checks say afterwards where the record and the work disagree. A part is
   not resolved, however it would end, while a Crux raised in it has no
   decision, and the parent waits for all of its parts.

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
- A Context whose behaviour its Contracts promise is based on them in the
  `change_set` that puts its first Contract (an entry
  `{"put":"base_on_contract","context":"<name>"}` beside the `contract` one),
  so that from then on it keeps one.
- An interpretation the text of the requirements, the repository or general
  knowledge settles is an Assumption of the part whose work it settles (the
  parent's, if it settles the request as a whole). Keep the requirements and their acceptance examples as
  files in the repository (a copy of what was given, if it came from elsewhere),
  so that an Assumption can cite them by path; the designer puts what an
  example shows into the Contracts (see [Roles](#roles)).
- A question nothing settles is a Crux with a recommendation, raised in the part
  it came up in (step 2). That part names the Context that holds the work its
  answer asks for, and the options name what choosing them would change. Once
  it is decided, the work its answer asks for, the Clause and the Cases that cite
  it, is applied `for` that part. The rest of the work, the Interfaces and the
  Harness too, is applied `for` the parts that ask for it (steps 3 to 6).

Each operation goes on one line, as `chidev exec` reads them:

```json
{"op":"init"}
{"op":"cr_open","title":"First version of tally","intent":"change_behavior","situation":"count named items from a list of commands","issuer":"the requester"}
{"op":"cr_open","title":"Add the tally Context","intent":"change_behavior","parent":"First version of tally","targets":[{"project":true}]}
{"op":"change_set","for":"Add the tally Context","puts":[{"put":"context","name":"tally"}]}
{"op":"cr_resolve","title":"Add the tally Context","resolution":"done","reason":"the Context is in","by":{"role":"orchestrator"}}
{"op":"cr_open","title":"Lines and counts of tally","intent":"change_behavior","parent":"First version of tally","targets":[{"context":"tally"}],"assumptions":[{"statement":"a command is + or - and a name, separated by exactly one space","reason":"the requirements give the form, and their example uses one space","evidence":[{"repo":{"path":"REQUIREMENTS.md"}}]},{"statement":"a line whose bytes are not text is a line that is not a command","reason":"the requirements say a line that is not a command of this form is not used","evidence":[{"repo":{"path":"REQUIREMENTS.md"}}]}]}
{"op":"change_set","for":"Lines and counts of tally","puts":[{"put":"contract","context":"tally","name":"lines","title":"Lines","clauses":[{"label":"C1","text":"a line is + or - and a name, separated by exactly one space"}]},{"put":"base_on_contract","context":"tally"}]}
{"op":"cr_crux_raise","in":"Lines and counts of tally","question":"What happens when a count would grow past the largest the program can hold?","kind":"ambiguity","importance":"normal","options":[{"choice":"the program stops with exit code 2","consequence":"input that takes a count past the largest gives no counts at all","recommended":{"reason":"no count it prints is wrong","confidence":60}},{"choice":"the count stays at the largest","consequence":"the other counts are printed, and that one is printed too low with nothing to say so"}]}
{"op":"cr_crux_decide","crux":"<crux id>","option":"<option id>","reason":"no count it prints is wrong","basis":{"arbitrary":"nothing in the Policy covers a count that passes the largest"},"by":{"person":"alice"}}
{"op":"change_set","for":"Lines and counts of tally","puts":[{"put":"contract","context":"tally","name":"lines","title":"Lines","clauses":[{"label":"C1","text":"a line is + or - and a name, separated by exactly one space"},{"label":"C2","text":"input that takes a count past the largest it can hold stops the program with exit code 2"}]}]}
```

`<crux id>` and `<option id>` stand for the ids that the result of
`cr_crux_raise` gives (the Crux's and the chosen option's); everything else is
written as it is.

The requirements (here `REQUIREMENTS.md`) say that a count never goes below 0
and nothing of how large it may grow. A program keeps a count in a number of
some size, and whether one past the largest stays there or stops the program
is two readings that nothing settles, so it is a Crux, raised in the part that
holds the counts. Until it is decided, `status` and `check` say so
(`cr.awaiting-decision`), the part cannot be resolved, and only the work that
depends on it waits; once it is decided (step 7), the part gets a Clause that
promises the answer (the last line). A line whose bytes are not text, by
contrast, is settled by the text: the requirements say a line that is not a
command of this form is not used, and that is such a line, so it is an
Assumption. The Interfaces, the Harness and the code of "Lines and counts of
tally" follow as steps 3 to 6, then the parts and the parent are resolved
(step 8).

## Never

- Add, change or retire a Context or an asset other than as the work
  (`change_set` `for`) of a request: a Context too is added, given its `uses`
  and based on its Contracts in a `change_set`, not with `context_add`,
  `context_uses` or `base_on_contract` on their own. The one edit that has no
  `for`, `items_rekey`, is run while the request it belongs to is open, and its
  resolution says so.
- Put a Harness from a role that has read the implementation.
- Decide a Crux in the name of a decision maker you are not (`by` is a claim
  that nothing checks, so it is kept true), or answer with a choice the Crux
  does not offer (an own wording is the decision maker's, not the role at
  work's).
- Pin code whose tests do not pass.
- Put a Harness again without changing the tests to make a stale citation go
  away: putting it says the Cases are written against the targets as they are
  now.
- Resolve a request as done when its work is not done; the record is what the
  next reader trusts.
