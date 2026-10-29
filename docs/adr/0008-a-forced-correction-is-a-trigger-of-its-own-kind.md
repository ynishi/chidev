# 8. A forced correction is a trigger of its own kind

Status: accepted

Amends [ADR 0002](0002-every-use-case-that-changes-state-is-a-trigger-and-one-door-keeps.md)
on one point: there are five kinds of trigger, not four. The rest of it holds: one
door keeps, and a use case that changes state enters through the entry of its kind.

## Context

A token, a password or other text that must not stay can go into records that only
grow:

- the title, situation and issuer of a change request, its Assumptions, and the
  reason and `by` of each resolution and reopening;
- the texts of a Crux and of its decision;
- the evidence of a measurement (the command and stderr of each step, its
  failure, a mutant's text, what a simulated case found);
- the record of a check's run (the command and stderr of an external program, the
  judge of a judged rule).

Each of them is append-only by its own rule, and no rule said when it could be
otherwise. The only way to correct one was to edit the database by hand.

The earlier attempt rewrote the database from the store itself, kept what it did in
a table of its own, and replaced a text everywhere unless told where. That left four
problems:

- it walked around the one door;
- it kept the correction apart from the record it corrected;
- it gave infra the domain's documents to edit;
- it changed what nobody had looked at.

None of the four kinds of ADR 0002 fits a correction:

- **An edit** runs the checks and can be refused by a mandatory check or an external
  gate. A gate that refuses a correction keeps the secret in the store, with no way
  left to remove it.
- **A record** cannot change a change request or a Crux: its type has none, which is
  what ADR 0002 made it for.
- **A confirmation and a question** do not change a record's text at all.

## Decision

**A forced correction is a trigger of a kind of its own** (`CorrectTrigger`, the use
case `correct_records`). It runs no check, asks no external program, and is refused by
no gate. What it may change is narrow:

- It changes only the texts of records, field by field, and each field is a type of
  its record (`ChangeRequestField`, `CruxField`, `MeasurementField`,
  `InvocationField`). A target, an expectation, a decision, a revision or a count
  cannot be named.
- Its entry in `app::commit` is given what to correct (the text, why, who, and per
  record the fields), reads those records from the store at the revision the use
  case read, and applies the domain's correction itself. So nothing reaches the store
  through it that a correction did not make, as nothing reaches it through an edit
  that the checks did not see.

**A correction is a concept of the domain, and a step of the record's own history.**
The text in each field named is replaced by a mark (`[redacted]`). The correction is
kept with the record: the fields, the reason, who and when, and never the text.

- A change request keeps it as a step of its history, beside its resolutions and
  reopenings. Those still alternate; a correction may stand anywhere and changes no
  state.
- A Crux, a measurement and a check's run keep a list of their corrections, which is
  where their history begins.
- A field that must stay distinct (a change request's title among the others, a
  choice among its Crux's) takes a numbered mark (`[redacted 2]`) when the plain one
  would make it equal to another. A uniqueness rule therefore never refuses a
  correction.

**Find first, then name each place.** A read-only use case (`find_text`) answers where
a text occurs (record and field). The correction names each place, and answers what
is left. Neither answer holds the text.

**The store clears what it kept of the old bytes.**

- `secure_delete` is on for every connection, so a row replaced by any commit leaves
  zeros, not its old content.
- After a correction is kept, the use case asks the store to write the file again
  (`VACUUM`), and to empty a WAL if there is one (`wal_checkpoint(TRUNCATE)`), through
  a port method of its own (`Store::clear_freed`). The commit itself does not look at
  the kind of trigger.
- A failure to clear is answered beside the kept correction, not as a failure of it.

## Consequences

- A correction is not told findings. No builtin check reads the texts it changes. The
  model's revision moves, because change requests and Cruxes are part of what an agent
  is shown, so a judged report made before it reads as stale. An external check sees
  the corrected state at the next `check_run`.
- Measurements and the record of runs are no longer append-only without exception: a
  correction rewrites their texts in place. A run keeps its place among the runs, so
  the latest stays the latest, and its stamp is not touched, so its standing does not
  move. The settings in a measurement's stamp are not a field: they are what its
  standing is read from, and they are a digest, so no text of `chidev.toml` is in
  them. A command's text reaches a measurement only in the evidence of a step,
  which is a field.
- The documents of a change request (schema 4) and a Crux (schema 3) are written in a
  form an older chidev refuses as newer than it reads; the earlier forms are still
  read.
- What the file system kept is out of reach: the rollback journal deleted after a
  write, the pages `VACUUM` cut from the end of the file, a snapshot, a backup, a copy
  in git. The guide's troubleshooting says what to do about those.

## Alternatives considered

- **Make the correction an edit.** The checks would run and a gate could refuse it,
  and a refused correction keeps the secret.
- **Keep the corrections in a table of their own.** The record would not say it was
  corrected, and a reader of a change request would have to know to look elsewhere.
- **Replace the whole record with a mark.** It is simpler, and it destroys what the
  rest of the record says. It also collides at once with the rules that keep titles
  and choices distinct.
- **Replace a text wherever it occurs when no place is named.** It changes what
  nobody saw first. Naming each place costs one `find_text` before.
- **Run `VACUUM` from the commit when the trigger is a correction.** The store would
  decide by the kind of trigger, and a failure after the correction was kept would
  come back as an error of the correction.
