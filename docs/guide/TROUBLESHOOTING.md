# Troubleshooting

What to do when something has gone wrong with a project's store. Like the rest of
the guide, this is a way of working, not part of chidev: when it and the
[README](../../README.md) or the RustDoc disagree, they win.

## Text that must not stay in the store

A token, a password, a customer's words or anything else that must not be kept
went into an operation, and chidev kept it. The store is the database the README
describes under
[The project and `chidev.toml`](../../README.md#the-project-and-chidevtoml); the
steps below go in this order.

### 1. Invalidate it first

If the text can be invalidated (a credential, a token, a signed URL, session
material), revoke or rotate it before touching the store. Correcting the store
contains nothing: whoever or whatever read the store before (a copy of the file, a
backup, an agent that ran `view` or `status`) has the text already, and only
rotation makes it useless. Once it is rotated, what follows is tidying, not
containment.

If it cannot be invalidated (a person's data, a third party's confidential
material), what is owed follows the content, not the file. chidev's own policy for
its repository, [After a disclosure](../../PUBLIC_DEVELOPMENT.md#after-a-disclosure),
sets out both cases and the reasoning, and the reasoning holds for a project's store
as well.

### 2. Find where else it went

The store is not the only place. The file of operations given to `chidev exec`, the
result lines it printed, a coding agent's transcript, a shell's history, a log, and
every copy of the store (a backup, the copy made before an upgrade) can hold the
same text. Nothing below reaches them; each is dealt with on its own.

### 3. Correct what the operations correct

What the store keeps, where, and whether an operation corrects it:

| What | Kept in (table) | Corrected by an operation |
| --- | --- | --- |
| The text of a Clause, the source of an Interface or a Harness, the text of a Policy line | `project`, one document holding the live model | Yes: put it again with the text corrected (`contract_put` or `clauses_put`, `interface_put`, `harness_put`, `policy_put`). The document holds the current text only. |
| The name of a Context or an asset, the label of a Clause, the key of an Item (its declaration), the name of a Case, once retired or dropped by a put | `project` (the record of what was retired) and `asset_names` | No. The record keeps names and revisions, not the text (see [Retiring](../../README.md#retiring)), and `asset_names` keeps the name of every asset, retired ones included. |
| The code a Pin accepted | `pin_codes`, one row per Interface (`pins` keeps a digest of it) | Yes: `pin` the corrected code; the row is replaced. |
| The latest result of an external check: its findings, the command it ran, the stderr it gave, its failure | `external_reports`, one row per rule | Yes: the next run of the check replaces the rule's row. |
| The command and the stderr of each run of an external check | `check_invocations` (column `external`), one row per check a use case ran | Yes, by `record_correct` (step 4). Otherwise appended, never changed. |
| What an agent reported for a judged rule: the judge, the messages of its findings | `judged_reports` and `judged_findings`, per rule | Yes: the next report for the same rule replaces the rule's report and its findings. The name of the judge is also kept in the `judge` column of `check_invocations`, which `record_correct` corrects (step 4). |
| A change request: its title, situation and issuer, its Assumptions (statement, reason, evidence), the reason and `by` of each resolution and reopening | `change_requests` | Yes, by `record_correct` (step 4). No other operation edits or removes them (see [Change requests](../../README.md#change-requests)). |
| A Crux: its question, its options (choice, change, consequence), the reason of the recommendation, the decision (wording, reason, what must still hold, the reason of an arbitrary basis, the reason a `beyond` was declined) and `by` | `cruxes` | Yes, by `record_correct` (step 4). A Crux that supersedes a decision adds one, and the old one stays (see [Crux, Policy and Assumption](../../README.md#crux-policy-and-assumption)). |
| A measurement: the command and the stderr of each step, the failure, each mutant's text before and after, and for each simulated case what it found, the situations it could not reach, why it could not decide, in the script's words, and why it could not start (its schedule and trace are kept as digests only) | `measure_runs` and `measure_items` | Yes, by `record_correct` (step 4); otherwise appended, never changed (see [Measurements](../../README.md#measurements)). Its inputs (`inputs`) hold digests, ids and revisions only: a command in `[measure]` reaches the store as part of a digest, and as text only in the evidence of a step. Not the name of a regress case or the key of a mutant, which key the item rows. |

Correct the live model first, then the Pins. Then run again each external check
that reported the text, and have each judged rule that reported it report again.
A text that was in the code or a Clause can also be in the records only
`record_correct` corrects: in a mutant's text, in what a simulation script wrote,
in the stderr of a run.

### 4. The records only a correction changes

A change request, a Crux, a measurement and the record of a check's run are
corrected by `record_correct`, after `text_find` has said where the text is. Both
are operations of `chidev exec` like any other; the file of operations holds the
text, so it is one more copy to deal with (step 2).

1. **Find it.** `text_find` lists every place (the record and the field) where the
   text occurs, exactly, case-sensitive, and never the text or what is around it:

   ```json
   {"op":"text_find","text":"<the text>"}
   ```

   The answer is a list of `places`, each naming a record by its id (a title can
   hold the text) and the field: for example
   `{"change_request":{"id":"…","field":"situation"}}`,
   `{"crux":{"id":"…","field":"recommendation_reason"}}`,
   `{"measurement":{"id":"…","field":{"step_stderr":0}}}`,
   `{"invocation":{"seq":12,"field":"stderr"}}`.
2. **Correct each place.** Give `record_correct` the places, as `text_find`
   answered them, with why and who:

   ```json
   {"op":"record_correct","text":"<the text>","reason":"a token got into the record",
    "by":{"person":"alice"},"places":[…]}
   ```

   - In each field named, the text is replaced by `[redacted]` and the rest of the
     field stays.
   - A title, or a choice within its Crux, that would become equal to another takes
     `[redacted 2]` (or the next number), so a correction is never refused for it.
   - Each record keeps the correction: a change request as a step of its history
     (`status` shows it with the step `corrected`), a Crux and a measurement as their
     list of `corrections`, and a run as a list the store keeps. `status` shows the
     corrections of a Crux, and of the latest measurement of each subject; those of
     an older measurement and of a run are kept, and no operation shows them. A
     correction holds the fields, the reason, who and when, never the text.
   - A correction names its fields by a word of its own (`situation`,
     `recommendation.reason`, `step[0].stderr`, `option[<id>].choice`,
     `assumption[0].reason`), where a place gives the same field in the form an
     operation reads (`"situation"`, `"recommendation_reason"`, `{"step_stderr":0}`,
     `{"option_choice":"<id>"}`, `{"assumption_reason":0}`).
   - The correction is forced: it runs no check, and no gate refuses it.
   - It is refused, with nothing changed, and a rule that names the record by its id
     and the field, never by a title:
     - the text is empty (`correct.text-empty`), or is part of the mark, such as `red`
       (`correct.text-in-mark`);
     - the text is in the `reason` or in the name given as `by`, both of which the
       record would keep (`correct.text-in-reason`);
     - no place is given (`correct.no-field`), or one is named twice
       (`correct.place-twice`);
     - a record is not found (`correct.unknown-record`), a field is not in its record
       (`correct.no-such-place`), or the text is not in a field named
       (`correct.text-absent`);
     - the field would no longer be a value of its kind once the mark is in
       (`correct.invalid-after`).
   - The answer has `corrected` (the places), `remaining` (where the text still
     occurs, as `text_find` would say) and `cleared` (step 5). When `remaining` is
     not empty, correct those places too. It is `{"unknown":"<error>"}` when the
     store could not be searched again after the correction was kept: run
     `text_find` to know.
3. **Check what is left.** Run `text_find` again; it should answer no place. It does
   not look at the live model, the Pins or the reports, which step 3 corrects.

What `record_correct` does not reach, a person can still edit by hand, outside what
chidev promises: the name of a regress case and the key of a mutant (the `key` column of `measure_items`), and the reason of a correction
itself. Make sure no `chidev` runs on the store, and make a backup first (`sqlite3
<store> ".backup <copy>"`). Replace the text with a mark, keep everything else as it
is, run `{"op":"status"}` and `{"op":"check"}` on the edited store, and go back to the
backup if either fails. Then clear what SQLite left (step 5), and destroy the backup or
keep it under the same protection as the text. Starting a new store, and building the
model in it again, is the other way; the change requests, the Cruxes and the history of
the checks do not come over to it.

### 5. What SQLite leaves in the file

Replacing a row does not by itself erase the old bytes: they can stay readable in
the free pages of the file and in the unused space of a page. chidev sets
`secure_delete` on every connection, so a row it replaces or deletes is overwritten
with zeros. After a correction, `record_correct` also writes the file again
(`VACUUM`), which removes what earlier versions of chidev, or an edit by hand,
left. When the file is in WAL mode, it then empties the `-wal` file (`PRAGMA
wal_checkpoint(TRUNCATE)`). Its answer says `"cleared":"done"`, or `not_finished` with
the error when either step failed. Another program that has the file open (a second
`chidev`, an `sqlite3` shell) can keep either step from finishing, and that is said
the same way. The correction is kept either way, and the clearing is then done by
hand, with the store not in use:

```sh
sqlite3 <store> "PRAGMA secure_delete = ON; VACUUM;"
```

Run the same after a store was edited by hand, or was used by a chidev older than
the one that sets `secure_delete`.

`VACUUM` writes the file again without its free space; the original pages it
copies to the `-journal` file while it runs, and the pages it cuts from the end of
the file, go back to the file system as they were. chidev uses SQLite's default
rollback journal: during a write the original pages are copied to a `-journal` file
beside the store, which is deleted when the write ends and left behind by a crash.
A `-wal` file beside it means something put the file in WAL mode; `PRAGMA
wal_checkpoint(TRUNCATE)` empties it, and it goes when the last connection to the
file closes cleanly. Once `sqlite3` has exited, neither file should be there.
What the file system keeps of a deleted file (a snapshot, a backup, the blocks
themselves) is out of SQLite's reach; step 1 is what contains that.

### 6. The store file in git

If the store file, or a `-journal` or `-wal` file beside it, was ever committed,
every commit that holds it keeps the text, whatever is done to the store now.
Rewriting that history is the repository's own business and done with its own
tools; chidev has no part in it. [After a disclosure](../../PUBLIC_DEVELOPMENT.md#after-a-disclosure)
says what a rewrite reaches and what it does not (other clones, forks, a mirror),
and why the rotation of step 1 comes first. A store that is not meant to be shared
belongs in a path the repository ignores.
