# 9. Environments are assets of a Context, and acceptance examples are a kind of Harness

Status: accepted

## Context

A Harness is of one of two kinds. A Harness of `tests` is run by the project's own
command, outside chidev. A `sim` Harness is a script chidev runs in a copy of the
project against the pinned code (see "Simulation" in the README). Both run in the
project's tree with the caller's permissions; the copy a measurement makes keeps the
working tree untouched and is not something a Case can observe or depend on.

That fits promises a test observes from inside: a function's result, a program's
output and exit code. It does not fit promises about what a program does around it,
which are often the ones a requester cares most about:

- it writes nothing outside its working directory;
- it leaves no process running after it ends;
- it opens no network connection, or only to the hosts it names;
- it works end to end against a real service (a database, a queue) as deployed.

To check one, a test needs an environment it controls and can inspect, defined before
the run and the same at every run. Nothing in the model can say that a Case runs in
one, so such a Clause cannot be cited by a Case that really checks it: it stays
uncited (`cite.uncited-clause`) or is reworded into something weaker.

Acceptance examples have the same gap from another side. An example of a command-line
program is data: the arguments, the standard input, the expected standard output
ending with a line `#exit <code>`, and optionally the expected standard error. The
guide has the designer put each example into a Clause that names its files, but the
files are outside the model: a change that touches only an expected output moves
nothing chidev sees, so the guide also has the designer reword the Clause by hand.

Two things are missing from the model, not from a runner:

- **What a run stands in** is not a value. With no name and no revision, a change of
  it can never be a change chidev sees, and a Case cannot claim to observe anything
  about it.
- **Examples as data** are not a kind of Harness. A Harness's evidence is code the
  project runs; an example is data a generic runner judges, and data outside the
  Harness moves no revision.

## Options

| option | what it does | rating |
|---|---|---|
| A. The runner only | A runner of examples built into chidev, registered as an external check; nothing else changes | 5/10: answers the first step for examples; environments untouched; a changed example file still unseen |
| **B. Assets, a Harness kind, measurements** | An Environment is an asset of a Context; a Harness names one; examples are a Harness kind; runs are measurements | **8/10**: answers both in the model's own terms, reusing shapes that exist |
| C. Narrow the requirement | Examples stay external; a project that needs a sandbox builds it in its own tests | 3/10: the environment stays invisible to chidev, which is the gap |
| D. Not now | Document the limits | 2/10: the runner is cheap and useful at once |

## Decision

**B**, delivered in slices, the first of which is A's runner, so the first change is
useful with the model as it is.

**Where an environment is declared.** An *Environment* is a record of a Context, put
and retired like a Contract and revisioned like one: a name, the provider that makes
it, a description the provider reads (an image reference, a sandbox profile), and what
it can observe (files written outside the working directory, processes left running,
network connections, a named service). A Harness names at most one, in a field
`environment`; a Harness without it runs in the project's tree as before, so every
stored Harness keeps its meaning. It is a field, not a kind of Harness, because an
environment is orthogonal to how a Harness is evaluated: a sim script and a set of
examples can both need a container. It is not only a project setting, because a
setting has no revision and is not in the model, so its change would be seen by a
settings digest at most, never by a Case or a Clause.

**Who provides it.** A provider program, as for Items and mutants: a protocol of its
own (`chidev-env 1`: prepare, observe after each case, tear down), with the command
named per provider in `[measure]`, so that registering a program to run stays
project-only. chidev does not implement sandboxes: they are specific to a platform and
do not belong in a pure core. It runs the protocol, records the evidence (the
provider's executable digest, as for every program it starts), and judges what the
provider reports.

**How a change of it is seen.** The Environment's revision moves when its description
or what it observes changes. It is on the basis of every run of a Harness that names
it, so a run made before the change is stale; the provider's executable is in the
settings digest, as every provider's is. A Case that checks a promise about the
environment cites the Clause like any other Case. An Environment is not on a Pin's
basis: a Pin stands on the Items its tests cite, and a run in an environment, like a
simulation, stands on the Pin rather than the other way round.

**Where acceptance examples sit.** First as an external check: `chidev accept`, a
runner of a directory of examples, which answers the external-check protocol with one
finding per failing example, about the Clause whose text names the example's files.
Then as a Harness kind `examples`: each Case is one example and holds its files in the
model, so changing an expected output is a new revision of the Harness and the Clauses
its Cases cite are seen to follow; `builds` names the Interfaces whose pinned code the
program is built from, as for `sim`.

**Which moments run them.** None. Running examples is slow, and a check reads records
and runs nothing, so a run is a record (`measure` of kind `examples`), as mutation and
simulation are. The checks that judge it run at `check`, and the ones that say the
latest run failed or is stale also at `pin` and `resolve`, since a confirmation must
not claim a state the latest run contradicts; none of them at `edit`.

**How runs are recorded.** As a measurement of kind `examples`, beside `mutate` and
`simulate`: its stamp is the Harness's revision (the files included), the digest of
the code pinned for each Interface it builds, the Environment's revision and the
settings digest; one item per example with its verdict and evidence. A correction
applies to it as to every measurement
([ADR 0008](0008-a-forced-correction-is-a-trigger-of-its-own-kind.md)).

## Slices

Each is one change, verifiable on its own:

1. `chidev accept`, the runner, with a face for a person (a line per example) and a
   face for an external check. No change to the model.
2. The Harness kind `examples`: the model holds the example files in its Cases, and
   a put is refused unless it has that shape.
3. `measure` of kind `examples`: chidev runs an examples Harness in a copy against the
   pinned code with the same judge as the runner, records the run, and the checks
   judge it.
4. The Environment record and the Harness field `environment`, on the basis of runs.
5. The provider protocol `chidev-env 1`, with what a provider observes recorded with
   the run and judged per cited Clause.
6. The guide: which Clauses need an environment, and how the designer and the Harness
   writer say so.

## Consequences

- Until slice 2, an example's files stay outside the model. The runner's finding
  names a Clause by a match of the file's path in the Clause's words, which is a
  translation at the edge of the external check, not a rule of the domain; the guide's
  rule to reword the Clause when a file changes stays until the files are in the model.
- The judge of one example (what it expects, what a program that ended answered, the
  verdict) is a value of the domain from slice 1, so the runner and the later
  measurement cannot judge differently.
- The vocabulary of what an environment observes is designed before a provider exists
  and may need a second version of its protocol; the record (slice 4) comes before the
  provider (slice 5) so that the model is settled first.
