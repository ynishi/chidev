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
| The command and the stderr of each run of an external check | `check_invocations` (column `external`), one row per check a use case ran | No: appended, never changed. |
| What an agent reported for a judged rule: the judge, the messages of its findings | `judged_reports` and `judged_findings`, per rule | Yes: the next report for the same rule replaces the rule's report and its findings. The name of the judge is also kept in the `judge` column of `check_invocations`, which no operation corrects. |
| A change request: its title, situation and issuer, its Assumptions (statement, reason, evidence), the reason and `by` of each resolution and reopening | `change_requests` | No: no operation edits or removes them (see [Change requests](../../README.md#change-requests)). |
| A Crux: its question, its options (choice, wording, consequence), the reason of the recommendation, the decision (wording, reason, what must still hold, the reason of an arbitrary basis) and `by` | `cruxes` | No: a Crux that supersedes a decision adds one, and the old one stays (see [Crux, Policy and Assumption](../../README.md#crux-policy-and-assumption)). |
| A measurement: the command and the stderr of each step, the failure, each mutant's text before and after, and for each simulated case what it found, the situations it could not reach and why it could not decide, in the script's words (its schedule and trace are kept as digests only) | `measure_runs` and `measure_items` (the name of a regress case is also the key of its item row) | No: appended, never changed, by design (see [Measurements](../../README.md#measurements)). |

Correct the live model first, then the Pins. Then run again each external check
that reported the text, and have each judged rule that reported it report again.
A text that was in the code or a Clause can also be in the records no operation
corrects: in a mutant's text, in what a simulation script wrote, in the stderr of
a run.

### 4. The records no operation corrects

There is no operation that corrects a change request, a Crux, an Assumption, a
measurement or a check's evidence; issue #44 asks for one. Until there is, a person
has two ways, and both are outside what chidev promises:

- **Start a new store.** Stop using the store, move the file aside (keep it under
  the same protection the text needs, or destroy it), and point `--store` or
  `[store] path` at a new one. Build the model in it again by putting it (`view` of
  the old store says what to put), pin again and measure again. The change requests,
  the Cruxes and the history of the checks do not come over; the file kept aside is
  the only record of them.
- **Edit the database by hand, at your own risk.** Make sure no `chidev` runs on the
  store, and make a backup first (`sqlite3 <store> ".backup <copy>"`). The records
  are JSON documents in the `doc` column of `change_requests` and `cruxes`, JSON
  text in the `failure`, `evidence` and `summary` columns of `measure_runs`, the
  `detail` column of `measure_items` and the `external` column of
  `check_invocations`, and plain text in the other columns (the `key` of an item
  row, the `judge` of an invocation). Replace the text with a mark (`[redacted]`), keep everything else as it is, and
  keep a text distinct where it names something among others (the title of a change
  request, the choice of an option within its Crux). chidev refuses a document it
  cannot restore as corrupt rather than repairing it, so run
  `{"op":"status"}` and `{"op":"check"}` on the edited store before using it, and go
  back to the backup if either fails. They read the change requests, the Cruxes and
  the latest measurement of each subject only, not older measurements or
  `check_invocations`; for those, check that the JSON is still valid (for example
  `SELECT seq FROM check_invocations WHERE NOT json_valid(external)`) and edit
  nothing but the text. Then clear what SQLite left (step 5), and
  destroy the backup or keep it under the same protection as the text.

### 5. What SQLite leaves in the file

Replacing a row does not erase the old bytes. chidev does not set `secure_delete`
and does not run `VACUUM`, so a row replaced or deleted by an operation or by hand
can stay readable in the free pages of the file and in the unused space of a page.
Clear it with the store not in use:

```sh
sqlite3 <store> "PRAGMA secure_delete = ON; VACUUM;"
```

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
