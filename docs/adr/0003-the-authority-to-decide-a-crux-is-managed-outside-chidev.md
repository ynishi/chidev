# 3. The authority to decide a Crux is managed outside chidev

Status: accepted

Builds on [ADR 0002](0002-every-use-case-that-changes-state-is-a-trigger-and-one-door-keeps.md),
which made the record of a change request an edit that an external gate sees.
This decision is about who may decide a Crux, and where that is kept.

## Context

A Crux is a question that the repository, general knowledge and the Policy do
not settle, so a decision maker answers it. The answer is an intentional,
arbitrary decision that cannot be reproduced, and the point of recording it is
who decided what, on which basis. That makes the authority to decide a part of
the model: the model's words have a name for it, `CruxAnswerable`, a decision
maker being granted to answer a kind of question.

The answer records `by`, a role or a person, and the domain has always said that
it checks nothing about the name. The question was whether chidev should go
further and keep the grants itself: a Project that says who may decide, and a
decision refused when its `by` holds none.

What settled it is what chidev can know about the caller. The measurement is a
read of what a command, a configuration file and the store hold:

- `by` is `{"role":<name>}` or `{"person":<name>}`, a name that is not blank. The
  operations that take it say who the caller claims to be; no operation, command
  line argument, configuration file or table of the store identifies who is
  calling.
- `chidev exec` is run by whoever can run a process and write the store file. It
  has no login and no session, and a configuration file holds `[store]`,
  `[checks]`, `[measure]` and `[external]`, never a user.
- A grant checked against the name the caller declares is satisfied by declaring
  a granted name. It would refuse an honest mistake and nothing else.

## Decision

**Who may decide a Crux is managed outside chidev, until chidev keeps the
identity of its users.**

- chidev records `by` as it was declared and checks nothing against it. A
  decision is as trustworthy as the process that runs chidev; whatever wraps it
  (a role that applies the Policy, a review step, a person at a terminal)
  decides who gets to call `cr_crux_decide`.
- The concept stays in the model's words and in the RustDoc of `DecisionMaker`,
  so that the boundary is written where a reader of the code and of the README
  meets `by`, and so that whoever connects chidev to an identity system knows
  what it is connecting.
- When chidev keeps user identities, a Project holds the grants (who may decide
  which Cruxes), and a decision whose `by` holds none is refused with the rule
  `cr.crux-not-granted`. That rule does not exist now, and nothing takes or
  stores a grant.

## Consequences

- Any caller can record any `by`. The guide says so and forbids deciding in the
  name of a decision maker one is not.
- No operation, table or rule is added for grants. When they are, it is a
  change of the store's layout and of the operations, and it refuses a decision
  that is accepted today: a breaking change that this record announces.
- A decision recorded before grants exist stays as it was declared. A rule that
  starts to refuse has to say what it does with them, and nothing here pre-empts
  it.
- `cr_resolve` and `cr_reopen` record `by` in the same way, as claims, and are in
  the same position: the boundary is the same for them.
- A project that wants a list of who may decide before then can keep one outside
  chidev: a program that reads `cruxes` is sent every decision with its `by`
  (see "External checks" in the README), and an external check that is a gate at
  the `edit` moment can refuse an edit for what it introduced. chidev ships no
  such check.

## Alternatives considered

- Keep grants in the Project now and check them against `by`. It enforces
  nothing, since the caller declares the name, and it adds an operation, a
  field in the store and a rule that would have to be kept compatible when real
  identities arrive. It would also tell a reader that authority is checked.
- Grant by role name only. The same: a role is a name the caller writes.
- Leave the concept out of the model and say nothing. A decision would then
  differ from a resolution only by its kind, and whoever wires chidev to an
  identity system would not know which word to give the authority.
- Read the identity from the environment (the user of the process, a variable).
  It is declared by the caller in the same way, and it ties the record to the
  machine the command ran on instead of to the person.
