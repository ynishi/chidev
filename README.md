# chi (知)

Contract, Interface and Harness as the assets of a codebase; the code is a cache pinned to them

chi is the project's name: the initials of Contract, Harness and Interface, and
知 (knowledge). The crates, the command and the repository are named `chidev`.

How a person or a coding agent works with it, from a request to pinned code, is in
the [guide](docs/guide/README.md).

## Install

```sh
cargo install chidev-cli
```

The package is `chidev-cli`, and the command it installs is `chidev`. A program that
uses chidev as a library depends on `chidev` (see "Crates").

## Crates

| crate | role |
|---|---|
| `chidev` | Umbrella: the name users depend on. Re-exports the core. |
| `chidev-core` | Core (SDK): `domain` (pure rules and types), `app` (use cases and their ports), `infra` (adapters with real I/O). |
| `chidev-cli` | The `chidev` command. |
| `chidev-provider-rust` | The Rust providers: of mutants (`chidev provider mutants-rust`, a pure function from a source text to edits) and of Items (`chidev provider items-rust`, from a source text to the keys and lines of its declarations). Depended on by the CLI only; the core reaches a provider as a process. |

The layer rules are in `arch-lint.toml` and run with `cargo test`. A crate or layer
that does not fit the project is deleted, together with its `arch-lint.toml` scope.

## The project and `chidev.toml`

A project's root is the nearest directory with a `chidev.toml` file, looking at
the directory `chidev` runs in and then at each parent. The file may be empty; it
marks the root. With none up to the top, `chidev exec` stops with status 2 before
any operation: it does not fall back to the current directory and it does not stop
at a `.git`. A parent's `chidev.toml` is found when a project has none, as Cargo
finds a parent's `Cargo.toml`. The files were `checks.toml` and
`.chidev/checks.local.toml` before; one of those in a directory on the way up to the
root (the root included) stops `chidev exec` with status 2 and the name to give it.

Paths inside an operation (`source_file`, `code_file`) and in the settings are
relative to the root, so they mean the same from any directory of the project; a
path on the command line (the file to run, `--store`) is relative to the directory
`chidev` runs in. The RustDoc of `infra::project` gives the reasons.

The settings are the tables of `chidev.toml`, each described where it belongs:
`[checks]` (the levels and parameters of the checks), `[external]` (see "External
checks"), `[measure]` (see "`[measure]` in `chidev.toml`") and `[store]`:

```toml
[store]
path = "var/chidev.db"   # relative to the root; an absolute path is kept as it is
```

The store is the database `--store` names; without it, the `[store] path` of
`.chidev/chidev.local.toml` (the person's own file, not kept with the project), then
the `[store] path` of `chidev.toml`, then `.chidev/chidev.db` under the root. The
person's file holds `[store]` and `[checks]` only: it can change a level or where the
store is, not register a program to run or a command to measure with. A table or a
key that is not known, or a table in a file that may not hold it, is refused with the
file named.

### The store's layout

The tables of the store have a layout, a number the file keeps (`PRAGMA
user_version`); it is 2. A file that is made now has layout 2. A file of layout 1
held a Crux inside its change request, and it is migrated once, when it is opened,
in one transaction:

- Each change request that held a Crux gives a Crux of its own. Its `raised_in` is
  that request, its `findings` are none (it was not raised from a finding), its
  `question` is the request's title, which stood for it, and its options are
  numbered in order. A recommendation and a decision that named an option by the text
  of its choice now name the option's id, and the decision has an id. A `supersedes`,
  which named a change request, names the Crux of that request. The change request
  documents are rewritten without their Crux. The revision of the store and of the
  model do not move: the model means what it did.
- What cannot be moved stops the opening, and the file is as it was: a text that is
  none of the options of its Crux, a `supersedes` that names a request with no
  Crux, a record that cannot be read, records that do not fit together afterwards.
  The error says what did not match.
- A request resolved otherwise than as `done` whose Crux was never decided is moved
  as it was, a record of what was; `cr.awaiting-decision` reports the Crux.
- What was saved before is stale once where the Cruxes are in what it was stamped
  over (see `view` under "Operations").

An older chidev refuses a file of layout 2 and says which layout it reads, and a
migrated file cannot be moved back, so copy the store before upgrading.

When a secret or other text that must not stay gets into the store, the guide's
[Troubleshooting](docs/guide/TROUBLESHOOTING.md#text-that-must-not-stay-in-the-store)
says what to rotate, what the operations correct, what they do not, and what
SQLite leaves in the file.

## Operations

`chidev exec <file> [--store <db>]` reads one JSON object per line, runs each as an
operation, and prints one result line per operation (`"ok"`, the `line` it came
from, and its fields). A line that fails gets `"ok":false` and `diagnostics` (see
"Diagnostics"), and the run goes on; the exit status is 1 if any line failed. Things are named, never
given by id: a Context by its name, an asset by its name within its Context, a
Clause by its label (`C1`), an Item by its key, a Case by its name. These are
the operations that build the model (the others are described below, where they
belong):

```json
{"op":"init"}
{"op":"context_add","name":"pricing","uses":["base"]}
{"op":"context_uses","context":"pricing","uses":["base"]}
{"op":"base_on_contract","context":"pricing"}
{"op":"contract_put","context":"pricing","name":"discount","title":"Discount",
 "clauses":[{"label":"C1","text":"a discount over 100% is refused"}]}
{"op":"interface_put","context":"pricing","name":"price","source":"fn price(total: u64) -> u64;\n",
 "items":[{"key":"price","lines":[1,1]}]}
{"op":"harness_put","context":"pricing","name":"price-tests","source":"#[test] fn refuses() {}\n",
 "cases":[{"name":"refuses","lines":[1,1],
   "items":[{"interface":"price","key":"price"}],
   "clauses":[{"contract":"discount","label":"C1"}]}]}
{"op":"pin","context":"pricing","interface":"price","code":"fn price(total: u64) -> u64 { total }"}
{"op":"status"}
```

- `init` makes the project (once: a second is refused, `project.exists`).
  `context_add` adds a Context; `uses` (optional) names the Contexts it uses,
  whose Contracts and Interfaces its Cases may cite.
  `context_uses` replaces that list. `base_on_contract` bases a Context on its
  Contracts (it needs one, and there is no way back: a second one is refused,
  `context.already-based`). The three are edits like a put is, and answer with
  `findings` as a put does (see "Triggers and moments").
- `contract_put` makes the Contract or revises it, whole: the `clauses` given are
  the Clauses it has afterwards, a label it had keeps its Clause, a label it
  did not have is new, and one left out is removed. A label is `C` and a number.
  The answer has the asset's `id` and `revision`.
  - A Clause is known by its id, and its label is the name a person uses for it, as
    an Item's key is. Its revision counts changes of its sentence only: a Clause
    given another label by `replaces` (`{"label":"C5","text":"..","replaces":"C2"}`)
    keeps its id, its place in the order and its revision, and so does the Contract,
    if nothing else differs, so the Cases that cite it stay current and nothing goes
    stale. The label replaced must not be given to a Clause of the same put
    (`input.replaces-kept`), is replaced by one Clause at most, and is a label the
    Contract has (`input.rename-conflict`, `input.unknown-name`). A label the Contract
    has, given by two Clauses, is `input.rename-conflict` too. A Harness put that
    still cites the old label is refused with the unknown clause
    (`input.unknown-name`); the citations already kept are by id and stay.
- `clauses_put` changes some Clauses of a Contract that exists and leaves the rest as
  it is:

  ```json
  {"op":"clauses_put","context":"pricing","contract":"discount",
   "clauses":[{"label":"C1","text":"a discount over 100% is refused"},
              {"label":"C4","text":"a discount is not given twice","replaces":"C2"},
              {"label":"C9","text":"a new promise"}],
   "remove":["C3"]}
  ```

  - A Clause in `clauses` whose label the Contract has gets the sentence given (the
    same sentence moves nothing). One with `replaces` takes the place of the Clause
    that has the old label, with the new label and the sentence, and keeps its id. A
    label the Contract does not have, without `replaces`, is a new Clause, after the
    Clauses the Contract has, in the order given. `remove` lists the labels of Clauses
    to remove; a Clause that a Case cites and that goes is refused as it is when
    `contract_put` leaves it out. The Clauses not named stay, in their order, and so
    does the title: to change the title, or to put the Clauses in another order, use
    `contract_put`. The Contract must exist (`contract_put` makes it).
  - It is refused, with nothing changed and every problem of the entry said, when a
    `replaces` or a `remove` is a label the Contract does not have
    (`input.unknown-name`, with the Contract's labels as `did_you_mean` and `field`
    saying which of the two), when the label a `replaces` names is also given as a
    label of the same entry (`input.replaces-kept`), and when a label is named twice:
    given by two Clauses, replaced by two, or removed and also given, replaced or
    removed again (`input.rename-conflict`, with `side` `label`, `replaces` or
    `remove`). A result with two Clauses of one label is refused by the domain.
  - The answer has the Contract's `id` and `revision`, as `contract_put` has. It is an
    edit with a trigger of its own (`put_clauses`), and in a `change_set` it is the
    entry `{"put":"clauses",..}` (see "Change sets").
- `interface_put` is the same for an Interface: `source` is its code, and each
  item has its `key`, the first and last `lines` it is on (from 1) and, when it is
  renamed, `replaces` (the key it had) to keep its id. Lines past the end of
  `source` are refused (`interface.lines-outside`).
  - The `key` is the name the Item is known by: a path such as `price` or
    `Stack::push`, one line, written by you or found by a provider (chidev does not
    read it; see `items` below). It is the Item's identity: an Item whose key is one
    the Interface has keeps its id, and a citation of it
    (`{"interface":..,"key":..}`) names it. Two Items of an Interface do not have
    one key (`interface.key-taken`); give them keys that tell them apart.
  - The declaration is not written: chidev takes it from the code. `lines` points
    at the declaration alone, from its first token (after a doc comment and an
    attribute) to the line that ends the signature, with `;` or the `{` of its
    body (for a `struct`, `enum` or `union`, the closing `}`: its fields and
    variants are what it declares); those lines, each without the white space at its end, are the Item's
    signature. A doc comment inside `lines` is part of it, so editing the comment
    revises the Item. Lines with nothing in them are refused
    (`interface.blank-item`).
  - A declaration that changes goes to the Item's next revision (and the
    Interface's), which makes the citations of it stale. A rename alone does not:
    an Item given another key by `replaces`
    (`{"key":"Stack::push","lines":[1,1],"replaces":"push"}`) keeps its id and its
    revision, and so does the Interface, if the code is the same, so the Cases that
    cite it stay current. The key replaced must not be the key of an Item in the same put
    (`input.replaces-kept`) and is replaced by one Item at most
    (`input.rename-conflict`).
  - `items` can be left out. A provider for the language of the Interface then
    finds them in `source` (see "Provider protocol `chidev-items 1`"): the language
    is the `lang` of the Interface's `[[measure.layout]]`, and the command is
    `[measure.items]` of `chidev.toml`. The keys and lines are the provider's, so
    nothing is written by hand, and a provider that fails or disagrees refuses the
    put before anything is read or kept. With `items` written, no provider is
    started. A put that leaves `items` out is refused when the Interface has no layout
    or no `lang` (`input.items-required`), when `[measure.items]` has no command for
    the language (`items.no-provider`), when the provider answers what the protocol
    or the code does not allow (`items.protocol`), and when it cannot be run to its
    end or does not answer within `timeout` (`items.failed`).
  - A key the provider gives differently (a path that moved, a renamed function) is
    a new Item, and the old one goes, unless the put says it is the same one:
    `renames` (only when `items` is left out) is a list of `{"from":"price",
    "to":"pricing::price"}`, and the Item that had the key `from` keeps its id, its
    revision and its citations under the key `to` the provider found. Only the keys
    named are renamed. It is refused when `from` is not a key the Interface had or
    `to` is not a key the provider found (`input.rename-unknown`), when a `from` or a
    `to` is named twice (`input.rename-conflict`), when `from` is still a key the
    provider found (`input.replaces-kept`), and when `items` is written as well
    (`input.renames-with-items`; with written `items`, `replaces` is how a key is
    renamed).
  - `status` shows the keys; write a key as it shows it. A store from before
    keys is read with each Item's old signature as its key: the lines of the
    signature, each without the white space at its ends, joined by one space (a
    signature that an Interface has twice gets ` #2`, ` #3`, ... from the second
    on). No revision moves, so nothing goes stale; `items_rekey` (below) brings
    them to the provider's keys, or put the Interface again with the keys you want,
    and `replaces` (the old key) to keep an id.
- `items_rekey` (`{"op":"items_rekey","context":..,"interface":..}`) gives the Items
  an Interface already has the keys and lines its provider finds now, with their ids
  and so their citations kept. It is how an Interface put by hand, or read from a
  store from before keys, comes to the provider's keys.
  - The provider is given the code the Interface **keeps**; the working tree is not
    read. An Item and a declaration the provider found are paired when their lines
    overlap and each overlaps no other on the other side. A paired Item takes the
    provider's key *and* its lines. An Item that is not paired stays as it is and is
    listed in `unmatched`: no declaration is on its lines, or it and the declarations
    share lines with others (two declarations on one line; an Item written over the
    lines of several).
  - The result has `rekeyed` (`{"from","to"}` of each key that changed), `revised`
    (`{"key","from","to"}` of each Item whose revision went up) and `unmatched` (keys),
    and the Interface's `revision` after it. A new key alone moves no revision. The
    signature is the text of the Item's lines, so an Item whose new lines declare
    something else goes to its next revision, and the Interface with it, and the
    Cases that cite it go stale; `revised` is that list.
  - It is refused, with nothing changed, when two Items would have one key
    (`interface.key-taken`: a key the provider gave twice, or one an Item that is not
    paired still has), and when the provider cannot be asked (`input.items-required`,
    `items.no-provider`, `items.protocol`, `items.failed`, as for a put). It is one
    operation of its own, not a `change_set`, and it is kept as the store stood when
    it began: a store that has moved since refuses it. It is an edit, so the answer
    has `findings`: a citation it makes stale (`cite.stale`) is one.
- `harness_put` is the same for a Harness, and `kind` is `tests` (the default for
  a new Harness) or `sim`. Each Case has its `name` and the first and last `lines`
  it is on in `source` (from 1): its test, an attribute above it included, or its
  case of a simulation; lines past the end of `source` are refused
  (`harness.lines-outside`). A Case of tests cites one Item at least and any Clauses;
  a Case of a sim cites Clauses only, and a sim Harness has `builds`, the Interfaces
  whose pinned code it runs (see "Simulation"). `context` in a citation (default:
  the Case's own) reaches a Context that one uses. A citation records the revision
  its target has when the Harness is put.
- `pin` records that the code was accepted: it needs a Case that cites an Item of the
  Interface, and it goes stale when what it stands on changes.
- The text of an Interface, a Harness or a Pin can be read from a file instead of
  being written into the line: `interface_put` and `harness_put` take `source_file`
  in place of `source`, and `pin` takes `code_file` in place of `code`. Exactly one
  of the two is given (both, or neither, is refused). The path is relative to the
  project's root (the nearest directory with a `chidev.toml`, where the settings
  are read from); an absolute path, a path with a `..` part, a file
  that is missing or cannot be read, and a file that is not UTF-8 text are refused
  with the path named. The file is read when the operation runs, and the model keeps
  the *text*, not the path: changing the file later changes nothing until the
  operation is run again. `lines` of an Item and of a Case are still given as they
  are.
- `status` lists the Pins and whether they hold, the citations behind their
  targets, the change requests, what was retired and the findings of every check.
- `check` runs every check on the project as it is and answers with the
  `findings`; nothing is kept. An edit cannot finish the model in one step, so
  a put answers only with what that put introduced (see "Change sets"): a Clause
  no Case cites yet, or a Context with no Interface yet, is not said by the put
  that left it so. The whole model is judged where something is confirmed (`pin`,
  `cr_resolve`) and by `check` and `status`. `check_list` lists every check with
  its `level`, its `kind` (`change` for a check that says an edit made something
  stale or out of place, `state` for one that says the model is not finished or
  not settled) and the `moments` it runs at.
  - `{"op":"check","context":"pricing"}` narrows the answer to one Context: the
    findings about it and about what it holds, and about a change request that is
    about it (a target it revises, retires or keeps is in the Context; the options
    of a question not yet decided name nothing). The run is the whole one and only
    what is answered is narrowed. `outside` counts the findings left out: those
    about the Project as a whole, about a change request that names nothing of the
    Context, and about something that is gone. It is in the answer only when a
    `context` was given. A name that is no Context's is `input.unknown-name`.
- `view` reads the part of the model you ask for, as the model holds it (the
  Clauses' text, the Interfaces' source and Items, the Harnesses' source and Cases):

  ```json
  {"op":"view","sections":["contracts","interfaces"],"contexts":["ledger"]}
  ```

  - `sections` is required and names at least one of `contracts`, `interfaces`,
    `harnesses`, `change_requests` and `cruxes`. There is no default that reads every
    section: leaving the field out is `input.missing-field` (`name` is
    `sections`), `[]` is `input.view-sections-empty`, and a word that is not a
    section is `input.invalid` (with `did_you_mean`).
  - `contexts` names the Contexts to read. Leaving it out reads every Context, `[]`
    reads none, and a name the Project does not have is `input.unknown-name`. A
    field `view` does not take (a misspelled `contexts`) is `input.unknown-field`,
    not a view of every Context. The
    change requests and the Cruxes belong to the Project, not to a Context, so they
    come whenever `change_requests` or `cruxes` is a section, whatever `contexts`
    says.
  - The result is the envelope of an operation (`ok`, `line`, `op`) with the
    `store_revision` the model was read at and the `view`. `view` is
    `{"version":4,"contexts":[..],"change_requests":[..],"cruxes":[..]}`: a Context
    always comes with its `id`, `name`, `basis` and `uses`, and with the sections
    asked for only. A section that was not asked for has no key; one that was and is
    empty is `[]`.
    `chidev schema view` describes this value of `view` (see "Protocol `chidev-exec 1`").
  - It is the part of the model that `check_rules` gives a judged rule and that an
    external check is sent, entry for entry, so a reader of a view sees what a rule
    is judged on. Asking for fewer sections is how a role is kept from the rest: an
    implementer that is to write code from the Contracts and Interfaces, and not
    from the tests that will check it, is given `["contracts","interfaces"]`, and
    nothing of the Harnesses (their source, Cases or citations) is in what it reads.
  - A change request comes with its `parent` (an id), only when it has one, but not
    with its `situation`, its issuer or its Assumptions: those are the request in the
    words of whoever asked and the readings of those words, and what is to be relied
    on of them is in the Contracts. It has no `crux` either: the questions that stop a
    request are the Cruxes raised in it, and they are the `cruxes` section. A Harness
    writer given the view learns which part a put is for and which request a question
    stops.
  - A Crux in `cruxes` comes with its `id`, `raised_in` (the id of the change request
    it stops), the `findings` it was raised from, its `kind`, `question` and
    `importance`, its `options` (each with its `id`, `choice`, `wording`, `expects` and
    `consequence`), the option `recommended` (by `option` id, with `reason` and
    `confidence`), `supersedes`, its `state` (`open` or `answered`) and, once
    decided, its `answer`: the `id` of the decision, the `decision` (`chosen` with the
    option id and the reason, `worded`, `unspecified` or `withdrawn`), its `basis`,
    `by` and `at`. The words of a Crux (an option's wording and consequence, the
    reason of the recommendation, a decision in other words) are given as whoever
    raised or decided it wrote them, so they say no more of the requirements than that
    person put there.
  - `cruxes` is a section an external check may read (see "External checks"). The
    stamp of a change request covers the Cruxes raised in it, and the stamp over the
    Project covers all the Cruxes. What was saved before the view was version 4 goes
    stale once and is judged or run again: an external result, since the model it
    was sent carries the version, and a judged finding about a change request that
    has a parent or a Crux, or about the Project of a store that has one. A stamp
    over a model with neither is what it was.

A put or a retire does not need an open change request, but it belongs to none: it
shows in the targets of no change request (only a `change_set` `for` the request
does, see "Change requests"). A change that touches
several assets and holds only as a whole is one `change_set` (next section).

### Triggers and moments

Every operation that changes what the store keeps is the trigger of one use case, and
the checks it is held to follow from the kind of use case it is. There are four
kinds, and a use case belongs to one of them for good:

- An **edit** changes what the model says: a put or a retire, a Context added or
  its `uses` set, the Policy, the record of a change request (`cr_open`,
  `cr_crux_raise`, `cr_crux_decide`, `cr_assume`, `cr_reopen`), `items_rekey`. The
  checks of the `edit`
  moment run on the model before it and after it, and the answer has `findings`:
  those the edit introduced. A mandatory check, or an external check that is a gate,
  refuses it only for those.
- A **confirmation** claims the model is in a state: `pin` (moment `pin`) and
  `cr_resolve` (moment `resolve`). The checks of its moment judge the whole model as
  it would stand, and everything they find is in `findings`.
- A **question** asks for every check on the model as it stands: `check_run` (moment
  `check`). It keeps what the checks said, external programs included, and refuses
  nothing, since it changes no asset.
- A **record** keeps a fact and runs no check: `init`, `measure`, `simulate` and
  `finding_report`.

The moments are `edit`, `pin`, `resolve` and `check`. They are what a check says it
runs at (`moments` in `check_list`, in a judged rule's file and in an external
registration), and what `check_rules` filters by (`moment`). The use cases of each
kind are listed where the types are, in the RustDoc of `chidev_core::app::commit`
(`Trigger`); a use case added there has to name its kind, and nothing is kept
that did not go through the entry of its kind. Each check run is kept with the use
case that ran it.

The moments were called `hooks`, `hook` in `check_rules`, and the moment `edit` was
`put`. Those words are refused, and the message names the one to write instead.

### Protocol `chidev-exec 1`

`chidev exec` is the protocol `chidev-exec 1`: it reads one operation per line and
prints one result per line, as above. `chidev schema` prints the JSON Schema
(2020-12) of the parts of it a program writes or reads, derived from the types that
read and write them, so a schema and the reading cannot differ:

```text
chidev schema          # lists the schemas, with the title and the id of each
chidev schema ops      # one line of exec: an operation. title "chidev-exec 1", $id urn:chidev:schema:ops:1
chidev schema view     # the `view` of the operation `view`. title "chidev-view 4", $id urn:chidev:schema:view:4
chidev schema external-call   # what an external check is sent (see "External checks"). title "chidev-external-call 2"
chidev schema external-reply  # what an external check answers. title "chidev-external-reply 2"
```

- **A schema is a necessary condition, not a sufficient one.** It says what a line
  must look like to be read. A rule that sets fields against each other is not in it:
  exactly one of `source` and `source_file`, exactly one of `option`, `wording`,
  `unspecified` and `withdrawn`, a subject, an item or a clause. An operation that breaks one is
  read, and refused when it runs, by a rule with a name (see "Diagnostics").
- The schema of the results of the other operations is not printed.
- The version of the view (`chidev-view 4`, `version` in the view, `VIEW_VERSION` in
  Rust) has its own promise: a field added does not change it, and only a change of
  meaning or shape (a field removed or renamed) does. A reader ignores a field it does
  not know and refuses a version it does not know.

## Change sets

Every rule of the Project is about the state a change ends in: a Case may cite only
what its Context owns or uses, a retirement may not take away what a Case that stays
cites. A put of one asset checks them on the Project that put leaves, so a change
that removes a Clause and an Item that Cases cite is refused whichever put comes
first, unless the Harness goes first, then the Contract, then the Interface, then
the Harness again. A `change_set` is that change as one operation: the rules are
checked once, on the state it ends in, and all of it is applied or none of it.

```json
{"op":"change_set",
 "puts":[
  {"put":"contract","context":"core","name":"stock","title":"Stock","clauses":[{"label":"C1","text":".."}]},
  {"put":"interface","context":"core","name":"stock","source":"fn reserve();\n",
   "items":[{"key":"reserve","lines":[1,1]}]},
  {"put":"harness","context":"core","name":"stock-tests","source":"..","cases":[
   {"name":"reserve_refuses","lines":[1,1],
    "items":[{"interface":"stock","key":"reserve"}],
    "clauses":[{"contract":"stock","label":"C1"}]}]}],
 "retires":[{"retire":"harness","context":"core","harness":"old-tests"}]}
```

- Each entry of `puts` has the fields of the operation of that name without `op`,
  and a `put` that says which: `context`, `contract`, `clauses`, `interface`,
  `harness`, `uses` (`context`, `uses`) or `base_on_contract` (`context`). Each entry
  of `retires`
  has the fields of the retire operation and a `retire`: `context`, `contract`,
  `interface` or `harness`.
- An `interface` entry takes `source_file` and a `harness` entry takes
  `source_file` in place of `source` (one of the two, as for `interface_put` and
  `harness_put`; the path is relative to the project's root, and the model keeps
  the text read, not the path). The files are all read before anything is applied;
  when several entries have a file that cannot be used, the set is refused with
  each of them named.
- `{"put":"clauses","context":"core","contract":"stock","clauses":[..],"remove":[..]}`
  changes some Clauses of a Contract that exists, as `clauses_put` does. A Case of a
  Harness in the same set can cite a label the entry adds or gives to a Clause (the
  Clauses it did not name are cited by their labels too), and what the entry moved is
  the Clauses whose sentence changed, the Clauses added or removed, and the Contract. A
  Contract named by a `contract` entry and a `clauses` entry in one set is named twice,
  and refused.
- `{"put":"context","name":"billing","uses":["ledger"]}` adds a Context, as
  `context_add` does (`uses` is optional; the Project goes to its next revision).
  It is applied first, so the same set can put the Context's first Contract,
  Interface or Harness, and a Harness can cite into it. It may use a Context the same
  set adds, whatever order they are listed in. A name that is taken is refused.
- The order the entries are listed in does not matter. They are applied in the
  order the rules need: the Contexts added, the uses of the Contexts, the Contracts and Interfaces,
  the basis, the Harnesses, then the retirements. A Case can cite a Clause or an
  Item that the same change set makes or revises, and the citation records the
  revision it has there; a Case that cites what the set removes fails the set.
  A sim Harness entry (`"kind":"sim"`) may carry `builds`, the names of Interfaces
  of its Context, which may be ones the same set makes; one that is not in the
  Context, or that the set retires while the sim still builds it, fails the set.
- What a put drops (a Clause, an Item, a Case) is recorded as retired, as it is
  for a put alone; the answer's `retired` lists it, then what was retired.
- Each asset moves by one revision at most: a thing is named once in a set, and
  naming it twice (or putting it and retiring it) is refused. An empty set is
  refused.
- When a set is refused nothing changes and the error names every problem found,
  in names: ``the case stock-tests release_frees (context `core`) cites the clause
  stock C7, which it cannot reach``. The checks of the `edit` moment run once on the
  Project before the set and once on the Project after it, and `findings` are
  those the set introduced: a finding the Project already had is not said again,
  and a mandatory check refuses the set only for what the set introduced. The
  same finding with a ground it did not have before (a Case that now cites one
  more thing that moved, a Pin that is stale for one more reason, a target moved
  again) is introduced; a finding whose grounds are all as they were is not. The
  store takes one commit.
- The answer has `puts` (each with its `put`, `context`, `name`, `id` and
  `revision`, in the order listed; a `uses` or `base_on_contract` has only the
  Context), `retired` and `findings`. A put or a retire alone is a change set of
  one and answers as it did.
- `for` is the title of a change request: `{"op":"change_set","for":"Remove release",
  "puts":[..],"retires":[..]}`. The work on a change request is applied as a
  change set `for` it. The request has to exist and not be resolved. The set records
  its footprint for the request in the same commit: the targets it moved, that is
  every target whose revision differs between the Project before and after (made,
  revised or gone: a Clause, an Item or a Case a put dropped, a thing retired, and
  the Contract, Interface, Harness, Context or Project whose revision moved because
  something in it was added or removed), each with the containers it was in. The
  answer is the same; `findings` includes `cr.out-of-frame` when the set left the
  frame (see "Change requests"; a Case that cites a Clause or an Item the frame
  holds is inside it, and the footprint keeps what each Case it moved cited). A
  set without `for`, a put or a retire alone, belongs to no request: it records
  nothing, shows in no request's `targets`, and
  is judged by revision only for the `keep` of requests. A Context added by
  `context_add` (and not by a set `for` a request) is such a move too.

## External checks

Register a project's checker in `chidev.toml`:

```toml
[external."ext.contract-consistency"]
command = "./tools/check-contract"
args = []
moments = ["edit", "check"]
timeout = 10
reads = ["contracts"]
gate = false

[external."ext.contract-consistency".params]
strict = true
```

The executable runs without an implicit shell, from the project's root (the nearest
directory with a `chidev.toml`). Bare command names resolve through `PATH`.
Registration is project-only; the person's `.chidev/chidev.local.toml` can change
policy, not register programs. A program has the caller's permissions: this is not a sandbox.

`chidev exec` accepts an explicit `{"op":"check_run"}` operation to execute
programs registered at the `check` moment and record their results. `status` and `check` never start
an external program; they show saved findings. `status.external` shows `unrun`,
`current`, or `stale`, comparing the supplied model sections and registration /
policy with the saved execution. `current` describes freshness, not success.
Results at `off` are omitted, and the program is not started. Scalar parameter
defaults are declared under the registration's `params`; existing `[checks]`
policy layers may override them with values of the same type.

At the moments `edit`, `pin` and `resolve`, the external checks registered for the
moment check the proposed model before it is saved (see "Triggers and moments" for
the operations that are held to each). Advisory findings are capped at warning.
`gate = true` makes every finding an error and refuses the change; execution and
protocol failures also refuse it. At `pin` and `resolve` every finding counts. At
`edit` the findings are held to what the edit introduced, as the builtin checks are,
when there is something to compare with: the saved result of the rule, if it
describes the model as it stood before the edit (the same input and registration,
which `status.external` calls `current`). The words of the registration that were
only renamed (`moments` was `hooks`, and `edit` was `put`) are counted as they
were, so a result saved before the rename is still `current`. A finding that result already had (the same subject and the same
message) is neither reported nor refused for; a program gives no typed reason, so
its message is what tells two findings about one subject apart. Without such a saved result every finding
counts, and a failed run refuses a gate whatever was saved. A gate cannot be
disabled or lowered by policy. A refused operation returns its findings and
execution evidence without saving them or the candidate change. Accepted changes
save their external results and invocation records in the same transaction.
`check_run` saves its results even when a gate finds an error, because it changes
no asset. A concurrent write refuses the commit, with no automatic process retry.

Programs receive one JSON document on stdin and return one on stdout. The request
is `protocol` 2: it has the `moment` the program is asked at (`edit`, `pin`,
`resolve` or `check`; version 1 called it `hook` and said `put` for `edit`), the
`rules`, `read_at`, `params` and the `model`. It does not say which operation asked:
a program is told when it is asked. (Which request a `cr_resolve` is resolving is
a concern of the builtin checks, which judge it though the model has it resolved;
it is not sent.) A response with no findings is
`{"protocol":2,"findings":[]}`; a response of `protocol` 1 is accepted as well (its
form did not change), and any other version is a failed run. Diagnostics
go to stderr. `reads` selects `contracts`, `interfaces`, `harnesses`,
`change_requests` and/or `cruxes`; unrequested sections are omitted, and a word that is not one, or
no word at all, is refused when `chidev.toml` is loaded. Context metadata is always
sent. The model sent is the `view` of the operation of that name, for every Context
and the sections read.

`chidev schema external-call` and `chidev schema external-reply` print the shape of
the request and of the reply. A reply is a `protocol` and its `findings`, each a
`rule`, a `message`, and, optionally, a `subject` (`{"kind":..,"id":..}`) and a
`level`:

```json
{"protocol":2,"findings":[{"rule":"ext.contract-consistency","message":"C2 contradicts C1",
 "subject":{"kind":"clause","id":"<the id of the Clause in the model sent>"},"level":"warning"}]}
```

What the shape does not say, and a reply is refused for (a failed run): the `rule` of
a finding is the rule the program was asked for (the one id in the request's `rules`,
the key it is registered under: `ext.contract-consistency` above); its `message` is
not blank; its `subject` is one present in the model sent. A finding with no
`subject` is about the whole project, and one with no `level` is at the level its rule
is held to; what a `level` becomes (capped for an advisory rule, an error for a gate)
is said above. See the RustDoc of `app::ports::external` for the wire types, and
`infra::external` for process limits and platform support.

The recorded SHA-256 identifies the executable read before launch. It does not
cover script arguments, dependencies, or later filesystem changes; displaying a
saved result does not re-hash the executable or check the filesystem. The check
input consists of the selected model sections, not a snapshot of the working
tree. Re-run a check after changing inputs outside that model.

## Measurements

A Harness that passes does not show it is strong. `measure` runs the pinned code
against changes of itself (mutants) and keeps what the Harness noticed:

```text
{"op":"measure","kind":"mutate","context":"inventory","interface":"stock"}
```

It is an operation of its own and a record, never an edit or a moment of the checks: it takes as long as the project's
build and tests take, once per mutant. It runs the code the Pin accepted (`pin`
keeps the text beside the Pin) and the Harness sources as the model holds them
now, in a copy of the project made under `.chidev/measure/<run>/`, with a target
directory of its own; the project's files and build output are never touched, and
the runner removes the copy when it is done (a killed process can leave one; it is
safe to delete). The working tree's own copy of the code is not read, so a tree
that has moved on does not change what is measured. Symbolic links are not copied.

The run builds and tests the copy once as it is (the *baseline*; if that fails the
run is a failed measurement and no provider is asked), asks the provider for
mutants, and for each one applies the edit, builds (a build that fails is
*stillborn*), tests (a failure *killed* it, a pass lets it *survive*, a deadline is
a *timeout*) and puts the code back. The result is kept as a measurement: its
stamp (the Pin's code digest, the revisions the Pin stands on, a digest of the
settings and of the provider program), each mutant with its site, operator and
outcome, the evidence of the commands (command, executable digest, exit code,
time), and the failure if there was one. Measurements are appended; the newest per
kind and Interface is current, the older stay as history
(`measure_runs`, `measure_items`). A result is kept in one commit with nothing else.

The checks read the record and run nothing (they are builtin, at the `check` moment):

| rule | default | finds |
|---|---|---|
| `measure.failed` | warning | the latest run failed (baseline, provider, protocol, timeout, budget, io) |
| `mutation.unmeasured` | note | a pinned Interface with no run |
| `mutation.stale` | warning | the Pin's code, the Interface, a Harness that checks it, a Contract it cites, or the settings moved after the run |
| `mutation.capped` | note | the run ended (`ok`) having evaluated fewer mutants than the provider found sites, `max_mutants` having limited it; about the Interface, `evaluated N of M sites` |
| `mutation.survived` | note | one per mutant no case noticed, about the Interface, with key, line, column, operator and the change |

A stale run is judged stale first, so a survivor of an old run is not reported for
moved code. `status.measurements` lists the latest run per Interface: stamp,
`sites` (the places the provider found), `evaluated` (the mutants run) and
`capped`, `killed` / `survived` / `stillborn` / `timeout`, `outcome` and `state`
(`current` or `stale`). The result of `measure` has the same `sites`, `evaluated` and
`capped`.

A run that did not look at every site says so. `max_mutants` limits what the
provider is asked for, so it samples the sites; the run is still `ok`, the mutants
it evaluated each have an outcome, and the record keeps `sites` (what the provider
found) apart from the mutants it was given (`planned`, stored in the summary of the
run) and the mutants evaluated. A run is *capped* when it ended with fewer mutants
than sites, which is `mutation.capped` (a note: a Harness that kills every mutant of
a sample may still miss the sites left out; raise `max_mutants` to cover them). A run
that stopped before it had evaluated the mutants it was given (the budget ran out)
is not capped: it is a failed run of kind `budget`, as before. A run that ended
with fewer mutants than it was given and no failure is refused as a measurement.

### `[measure]` in `chidev.toml`

```toml
[measure]
build = ["cargo", "build", "--quiet", "--tests"]   # a failure makes the mutant stillborn
test  = ["cargo", "test", "--quiet"]               # all pass: survived; a failure: killed
copy_exclude = [".git", "target", ".chidev"]       # relative to the root; the default
env = { CARGO_TARGET_DIR = "{run}/target" }        # default; {run} is the run's directory
timeout = 30            # seconds, one build or one test of a mutant
baseline_timeout = 600  # the build and the test before any change
budget = 900            # the whole run, at most 3600
max_mutants = 40        # at most this many mutants are evaluated; more sites make the run capped
seed = 0                # moves where the even sampling of sites starts
operators = ["rel", "eq", "arith", "logic", "bool", "int", "str"]  # optional; default: every class the provider makes

[measure.mutants]       # language -> the provider command
rust = ["chidev", "provider", "mutants-rust"]

[measure.items]         # language -> the command that finds the Items of an Interface
rust = ["chidev", "provider", "items-rust"]

[[measure.layout]]      # where an Interface's code is in the tree
context = "inventory"
interface = "stock"
lang = "rust"           # selects the providers (of mutants, and of items)
path = "src/stock.rs"

[[measure.layout]]      # where a Harness's source is written
context = "inventory"
harness = "stock-tests"
path = "tests/stock.rs"
```

`build` and `test` are commands, run without a shell in the copy; paths are
relative to the project root and may not contain `..`. Every Harness that checks the
Interface (the Harnesses of the Pin's basis) needs a layout entry, and the Harness
must be a file of its own (an in-file `#[cfg(test)]` module cannot be told apart
from the code). A field that is not known is refused by name. A project with no
`[measure]` has no runner: `measure` is refused, and the checks still judge what is
kept. The settings digest covers the whole table and the provider's executable, so
changing a command or upgrading the provider makes earlier runs stale.

**Placeholders.** A run has two places, and each has a name of its own that means the
same wherever it is taken: `{run}` is the run's directory (under `.chidev/measure/`,
removed when the run ends), and `{tree}` is the root of the copy of the project the
run works in, which is the `tree` directory inside it. A sim makes a second, fresh
copy for the second runs of its cases (`rerun-tree`, in the same run directory), and
there `{tree}` is the root of that copy while `{run}` stays the same: what is under
`{run}` (the build's target) is shared by the first and the second runs, so state the
second runs must not inherit goes under `{tree}`. Only two kinds
of field take a placeholder: the value of a variable of `env`, and a word of the
`program` of a `[measure.sim.<context>]`. Any other `{...}` in them is left as it is.
A `{run}` or `{tree}` in any other string of `[measure]` (a command, a layout `path`,
`copy_exclude`, a key) is refused by name, and so is `{work}`, which was the name of
both places before each had one: a configuration that still has it stops every
command until it is rewritten.

`operators` limits the classes of mutant the provider is asked for (the bundled one
makes `rel`, `eq`, `arith`, `logic`, `bool`, `int` and `str`). Leave it out for all of
them; an empty list is refused. It is part of the settings digest, so changing it
makes earlier runs stale. Leave `str` out to skip the mutants of string literals.

`[measure.items]` is read by the same loader as `[measure.mutants]`: a language and
the command that finds the Items of an Interface of that language (see "Provider
protocol `chidev-items 1`"). The language of an Interface is the `lang` of its
`[[measure.layout]]` entry, and a `lang` is valid when `mutants` or `items` (or both)
has a command for it. Three things follow from the table being `[measure]`'s:

- A project with no `[measure]` table has no provider of items: `interface_put`
  and a `change_set` cannot leave `items` out there (`items.no-provider`), and write
  them. A `chidev.toml` with other tables but no `[measure]` is the same.
- A language with a command in `items` alone is valid, and it can be measured by
  nothing: `measure` stops, naming it, with "`mutants` has no command for `<lang>`".
- The provider is started in the project's root, as the provider of mutants is, and
  must answer within `timeout`. The settings digest covers the whole table, so a
  changed `items` command makes earlier runs stale too.

### Provider protocol `chidev-mutants 1`

The provider is a program that makes mutants and nothing else: a pure function
of a source text, so a provider for another language is small. One JSON
document on stdin, one on stdout, stderr for diagnostics:

```json
{"protocol":1,"path":"src/stock.rs","source":"...","max":40,"seed":0,"operators":["rel","str"]}
{"protocol":1,"sites":118,"mutants":[{"key":"reserve/rel/2/9f3a","line":42,"col":13,
  "operator":"rel","span":[1834,1835],"from":"<","to":"<="}]}
```

A mutant is an edit: `span` is a byte range of `source`, `from` the text it holds,
`to` what replaces it. chidev checks `source[span] == from` for every edit before
any is applied; a disagreement, a key given twice, a protocol other than 1, or a
field the protocol does not have is a failed run (`protocol`). `key` is the
provider's stable id of the mutant (the bundled one writes
`<function>/<operator>/<n>/<digest of the line>`, which an edit elsewhere in the
file does not move); chidev treats it as opaque text. The reply is capped at 4 MiB.
The bundled Rust provider is `chidev provider mutants-rust`: it mutates operators
(`rel`, `eq`, `arith`, `logic`), boolean literals (`bool`), integer literals
(`int`) and string literals (`str`) inside function bodies, leaves `#[cfg(test)]`
and `#[test]` code alone, and gives the same reply for the same request.

The request field `operators` (optional, a list of class names; absent means all)
selects the classes. It is optional, so the protocol stays version 1 and a request
without it parses as before; a provider that does not know it may ignore it, and
the bundled one refuses a name that is not one of its classes. `[measure]
operators` is what chidev sends.

A string literal (`"..."`, `r"..."`, `r#"..."#`) gives two mutants, both of class
`str` and told apart in the key (`str-empty`, `str-marker`): the text emptied, and
`*` appended inside the quotes. Every `{...}` placeholder is kept in the emptied
text (`"{name}: {n:>w$} left"` becomes `"{name}{n:>w$}"`), so a format string still
builds. Not mutated: byte and C strings, strings in attributes, strings in the
arguments of `include_str!`, `include_bytes!`, `include!`, `env!`, `option_env!`,
`concat!`, `stringify!`, `compile_error!`, `cfg!`, `asm!` and `macro_rules!`. The
point of the class is the message no test reads (a wrong number or an empty text
in an error): such a mutant survives.

### Provider protocol `chidev-items 1`

The provider of items is a program that finds the declarations of an Interface in
its code and nothing else: a pure function of a source text. Like the provider of
mutants it gets one JSON document on stdin, answers one on stdout, and writes its
diagnostics to stderr:

```json
{"protocol":1,"source":"pub fn price(\n    total: u64,\n) -> u64;\nfn tax() -> u64;\n"}
{"protocol":1,"items":[{"key":"price","lines":[1,3]},{"key":"tax","lines":[4,4]}]}
```

An Item is a `key` (the name the Item is matched by when the code is put again: not
empty, one line, opaque to chidev) and the first and last of its `lines`, from 1,
that point at the declaration alone, from its first token to the line that ends the
signature. There is no `kind`. chidev checks the reply against the code before it
puts anything: a protocol other than 1, a field the protocol does not have, an empty
key, and `lines` that are not lines of the code are `items.protocol`; a command that
cannot be started, exits with a failure, or does not answer within `timeout` is
`items.failed`. Whether the declarations are apart is not checked: a trait and its
method on one line have the same `lines`, which is a right answer, and the Items
that share a line are all revised when that line changes. The same key twice is
refused by the Interface (`interface.key-taken`). The reply is capped at 4 MiB.

The bundled Rust provider is `chidev provider items-rust` (`rust = ["chidev",
"provider", "items-rust"]` in `[measure.items]`). It finds a `fn`, `struct`, `enum`,
`union`, `trait`, `type`, `const`, `static` and `macro_rules!` item, and the `fn`,
`const` and `type` inside an `impl` or a `trait`; an inline `mod` is entered, and
nothing in a function body is found. A declaration with no body
(`fn price(total: u64) -> u64;`, the form the examples above use) is found as well. The
`lines` of an Item run from its first token, after the doc comments and attributes, to
the line that has the `;` or the `{` of its body; a `struct`, `enum` or `union` runs on
to its closing `}`, so a field or variant added, removed or retyped revises the Item.

A key is a name and a path, not the text of the declaration:

| where | key |
|---|---|
| free, or in `mod m` | `price`, `m::price` |
| a `macro_rules!` | `twice!` |
| in an inherent `impl` | `Stock<T>::new` (the arguments of the type are in it: `impl Foo<u8>` and `impl Foo<u16>` differ) |
| in a `trait Clock` | `Clock::now` |
| in `impl Display for Money` | `<Money as Display>::fmt` |

A type that is not a path is put in angle brackets (`<[u8]>::len`), and inside a `mod` the
key is led by the path of the `mod` (`m::Stock::new`, `m::<Money as Display>::fmt`).

A key is an identity, not a display, so it is **not written as rustdoc writes it**.
It is printed from the syntax of the code by two rules with no exception: a list has
its `,` between the elements only (a trailing `,`, and so a formatter, a line break
or white space, moves no key; a tuple of one element, `(u8,)`, keeps its `,`, as
`(u8)` is another type), and a space stands between two words (identifiers, keywords,
lifetimes, literals) and nowhere else. A path is written as it is in the code, and a
lifetime is kept. So `<Box<dyn Error> as From<&str>>::from` is the key
`<Box<dyn Error>as From<&str>>::from`, `fn() -> u8` is `fn()->u8`, and `&'a mut T`
stays `&'a mut T`. Do not type a key from memory or from rustdoc: copy it from
`status` or the model, where chidev shows it. A key made some other way (by hand, or
the signature of a store from before keys) is brought to this one by `items_rekey`.

What to know about these keys:

- Two declarations that the key cannot tell apart get one key and are refused
  (`interface.key-taken`): the same name in two namespaces (`struct Foo` and
  `fn Foo`), and declarations told apart only by `#[cfg]` (`#[cfg(unix)] fn f()`
  and `#[cfg(windows)] fn f()`). Put such an Interface with its `items` written.
- A key follows the name: a declaration that moves to another `mod` or is renamed has
  a new key, so it is a new Item unless the put says otherwise (`renames`). The names
  of the generics of an `impl` are in the key as well: `impl<T> Foo<T>` changed to
  `impl<U> Foo<U>` is a new key, and a `renames` carries the id over.
- Two declarations on one line (`pub trait Clock { fn now(&self) -> u64; }`) have
  `lines` that overlap, which is a right answer. Both are revised when that line
  changes, and `items_rekey` leaves them in `unmatched`. Put them on lines of their
  own (a formatter does) or write the `items`.

### Simulation (blind, any language)

Tests written next to the code tend to check what the code does. A *sim Harness*
checks the code against the Contracts only: a script, written without reading the
program, drives the program as a black box with a schedule it makes from a seed,
and says which Clause did not hold. chidev runs it; it is the same for a Python,
TypeScript or Rust program, because chidev only starts processes and reads their
output.

A Harness has a kind: `tests` (the default; what every Harness has been) or `sim`.

```text
{"op":"harness_put","context":"ledger","name":"bank-sim","kind":"sim","builds":["bank"],"source":"...","cases":[
  {"name":"sweep","lines":[1,1],"clauses":[{"contract":"account","label":"C1"},{"contract":"account","label":"C2"},{"contract":"audit","label":"C1"}]},
  {"name":"regress-overdraw","lines":[2,2],"clauses":[{"contract":"account","label":"C2"}]}]}
```

A sim Harness has exactly one Case named `sweep` and any number named
`regress-<name>` (`a-z`, `0-9`, `-`). The `sweep` stands for the seeded runs and
cites every Clause the script checks; each `regress-<name>` stands for a fixed
schedule the script keeps. A sim Case cites **Clauses only**, of any number of the
Contracts of this Context (the example takes on `account` and `audit`, which both
have a `C1`); it cites no Item, and a Case that does is refused. Without `kind`, a
new Harness is of tests and an existing one keeps its kind; changing the kind is a
new revision. A Context has at most one sim Harness.

`builds` names the Interfaces of the Context whose pinned code the program is built
from. It is not a citation: it asks no Item of the Interface, a name the Context has
no Interface of is refused (also in a `change_set`, where the Interface may be one
the same change set makes), and an Interface cannot be retired while a sim Harness
builds it. Only a sim Harness has `builds` (a Harness of tests with it is refused),
a sim Harness builds at least one Interface (a put without one is refused; the rule
is found with `{"op":"explain","rule":"sim.builds-none"}`), and a
put states the whole list, as it states the code. Because a sim cites no Item, it is
on no Pin's basis: putting or changing the sim never makes the Pin of an Interface
stale. The Pin of an Interface still needs a tests Harness whose Case cites one of
its Items (code nothing checked is not accepted). The other way round, the
simulation stands on the Pin: the Harness (its script, Cases and `builds`), the
Contracts it cites a Clause of, and each built Interface with the digest of its
pinned code are its basis, so a change to any of them makes the latest simulation
stale (`sim.stale`).

```text
{"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"}
```

It copies the project under `.chidev/measure/<run>/` as a mutation run does,
writes the code the Pins of the Interfaces the Harness builds accepted and
the script (the Harness's source as the model holds it) over the copy, and runs
`build`. A program that does not build is a failed run (`no_build`) and the script
is not run. Then it starts the script once per case, `seed-0` to `seed-<seeds-1>`
and every regress case, and then runs `seed-0` and up to `sample` of the cases that
had findings once more, in a second, fresh copy.

#### Protocol `chidev-sim 2`

```text
<interpreter...> <script> <case> --out <dir> -- <program argv...>
```

`<case>` is `seed-<n>` or `regress-<name>`; the working directory is the copy; the
program argv is `program` of the configuration with `{tree}` made the root of the
copy (and `{run}` the run's directory; see "Placeholders"). The script drives the
program (standard input and output, arguments), writes `<dir>/schedule` (the
choices it made from the case) and `<dir>/trace` (what the program did under them),
and prints, one per line:

```text
FINDING <case> step <k> <contract>:<label>: <text>
UNREACHED <contract>:<label>: <text>
UNDECIDED <text>
CANNOT_START <text>
sim: findings=<n>
```

- A Clause is named `<contract>:<label>`: the Contract's name as chidev knows it
  (no white space; the name is read up to the first white space, the label is
  after its last `:`) and the Clause's label (`C<n>`) in it. Labels are unique only
  within a Contract, so a name without the Contract (`C1`, as `chidev-sim 1`
  wrote it) is refused. The script declares nothing else; the version is the
  protocol this document describes. chidev resolves each name to the Clause's id,
  among the Clauses the `sweep` Case cites, when it records the run: **the record
  holds Clause ids, not names or labels**, and the rules read ids, so a label
  renumbered later does not move an old finding. A new label moves no revision (see
  `contract_put`), so `sim.stale` does not warn ahead: a script that still prints the
  old label is a protocol failure on its next run, and the label to print is the one
  `status` and `view` show.
- `FINDING`: after step *k* (from 1) of this case a promise of the Clause did not
  hold. The case must be the one the script was started for.
- `UNREACHED`: this case never reached the situation the Clause needs. A Clause is
  *unreached* for the run when every seed that ended normally said so.
- `UNDECIDED`: the script could not decide something; counted, never a failure.
- `CANNOT_START`: the script could not start the program at all (the program is not
  there, it cannot be run), so it drove nothing and checked nothing. It is the script
  that sees this, because it is the one that starts the program; chidev only starts
  the script. The report is then this one line and `sim: findings=0`: a `FINDING`,
  `UNREACHED` or `UNDECIDED` line beside it is a protocol failure, for it would tell
  of a run that did not happen. The case is recorded as `cannot_start:<text>`; it
  has no findings and says nothing of any Clause (it does not count towards
  *unreached*, and it is not run again). A script that prints a `FINDING` for each
  Clause because the program did not start makes every Clause look broken by a
  program that was never run; print `CANNOT_START` instead. Exit 0 or 1, as for any
  report that is read.
- `sim: findings=<n>` is the last line and `n` is the number of `FINDING` lines.

When *every* case that was run said `CANNOT_START`, the run is a failed run of kind
`no_start` (the cases are kept, there are no second runs), judged by `sim.error`
alone: no `sim.finding` and no `sim.unreached` come of it. When only some cases did,
the run is `ok`, the others are judged as usual, and `sim.error` names the cases
that could not start.

Exit 0 or 1 (1 when there are findings) means the report is read. A name of a
Contract the sweep Case cites no Clause of, or of a Clause of a Contract it cites
that the sweep does not, a name that is not `<contract>:<label>`, a line of none of
these forms, a missing or repeated
summary, a summary that disagrees with the lines, a normal case that wrote no
`schedule` and `trace`, or a report over 1 MiB is a protocol failure of the whole
run; the cases run so far are kept. Another exit code, or a case past `timeout`,
is recorded for that case and the run goes on. chidev takes the SHA-256 of
`schedule` and `trace` itself, and it is chidev, not the script, that runs the
sampled cases again: a different schedule means the script draws something besides
its seed (a fault of the script), the same schedule and a different trace means
the program draws its own order (a fault of the program).

#### `[measure.sim.<context>]` in `chidev.toml`

```toml
[measure]
build = ["python3", "-m", "py_compile", "bank.py"]   # a failure: the run fails with no_build
# `test` and a layout `lang` are only for mutation; a project that is only simulated needs neither

[measure.sim.ledger]
interpreter = ["python3"]                  # argv prefix; the script path follows it
program = ["python3", "{tree}/bank.py"]    # the program under test; {tree} = root of the copy
seeds = 50       # seed-0 .. seed-49 (default 50, at most 10000)
sample = 6       # cases with findings run again besides seed-0 (default 6, at most 1000)
timeout = 60     # seconds, one case (default 60)
budget = 600     # seconds, the whole run, second runs included (default 600, at most 3600)

[[measure.layout]]
context = "ledger"
interface = "bank"          # the code of each built Interface is written here
path = "bank.py"

[[measure.layout]]
context = "ledger"
harness = "bank-sim"        # the script
path = "sim.py"
```

Every Interface a sim Harness builds needs a layout entry and a Pin (`pin` keeps
the code text); the script needs one. `build` runs with `baseline_timeout`. The
settings digest covers the whole `[measure]` table and the interpreter's
executable.

#### Rules

A put of a sim Harness is refused unless it has the shape above (the `sim.*` rules
that are invariants; `{"op":"rules"}` lists them and `{"op":"explain"}` gives each).
To replace the sim Harness of a Context, put the new one and
retire the old one in the same change set. The shape is asked of the Harness a put
names, so a store that holds a sim Harness from before the rule is read as it is, and
only a put of that Harness is refused; `sim.shape-kept` reports it.

| rule | default | finds |
|---|---|---|
| `sim.shape-kept` | error | a sim Harness kept in a shape a put of it is refused for: one the store held before the rule, or one beside another sim Harness of its Context; about the Case, the Clause or the Context (moment `check` only; it does not refuse a change). One label in two Contracts of the sweep is fine |
| `sim.uncited-clause` | warning | a Clause that no sim Case cites, of a Contract a sim Harness takes on (cites at least one Clause of); a Contract the sim cites nothing of is left to the tests and not reported |
| `sim.unrun` | warning | a sim Harness never simulated |
| `sim.stale` | warning | the script, a Case or the `builds` of the Harness, a Contract it cites a Clause of, a built Interface, the code pinned for it, or the settings moved after the run |
| `sim.finding` | error | one per Clause with findings, about the Clause, with the first case and step |
| `sim.unreached` | error | one per Clause no seed reached, about the Clause |
| `sim.nondeterministic` | error, **mandatory** | a case that gave another schedule or trace the second time, about the Case; no policy lowers it |
| `sim.error` | warning | the run failed (`no_build`, `no_start`, protocol, budget, ...) or the script failed for a case, or could not start the program in it (`CANNOT_START`) |

A stale run is judged stale first (only a second run that did not reproduce is
still reported). A failed run is judged by `sim.error` alone. `status.measurements`
lists the latest simulation per sim Harness with `harness`, `sim` counts (`cases`,
`findings`, `unreached`, `abnormal`, `reruns`, `differing`), `outcome` and `state`.

#### Writing a sim script (for the procedure that writes it)

The script is written blind, from the Contracts and the Interfaces only. It never
reads the program's code, its tests or its history, so that it checks the promises
and not the implementation. Therefore:

- cite in the `sweep` Case every Clause the script checks, and print a `FINDING`
  or `UNREACHED` only with the name `<contract>:<label>` of a Clause it cites
  (the Contract is part of the name, because two Contracts may both have a `C1`);
  a script may check the Clauses of several Contracts of the Context;
- when starting the program fails, print `CANNOT_START <text>` and `sim: findings=0`
  and nothing else; never a `FINDING` per Clause for a program that did not run;
- check each Clause literally, with an expected value computed from the Clause's
  sentence by the script's own model, never from the program's answers;
- use only the seed for randomness (`random.Random(seed)`, not the clock, the
  process id, the environment, unseeded random numbers, or the iteration order of an
  unordered collection), so that the same case gives the same `schedule` every time;
- write everything the program did into `trace` in a stable order, and make a
  schedule that reaches the situations the Clauses need (print `UNREACHED` when a
  case does not), keeping a `regress-<name>` Case for each situation that is hard to
  reach by chance or was once a finding.

## Change requests

A change request says what is asked, why (`situation`, in the words of whoever
raised it, and the `issuer`: both are kept and `status` shows them), which
targets it asks to have revised or retired (`targets`) and which to keep as they
are (`keep`). It may carry the `assumptions` made while raising it, and it is stopped
by the Cruxes raised in it, choices a decision maker has to make (see "Crux, Policy
and Assumption"); it holds none itself. A request is **closed by a
decision a person or a role takes and records** (`cr_resolve`), never by what
the definitions did: the targets are the content of the request, not a condition
of closing it. The targets are the assets, as a tree, and each has a revision:

```text
Project
 └ Context
    ├ Contract  ─ Clause
    ├ Interface ─ Item
    └ Harness   ─ Case
```

A reference names a target: `{"project":true}`, `{"context":"pricing"}`,
`{"contract":{"context":"pricing","contract":"discount"}}`,
`{"interface":{"context":..,"interface":..}}`, `{"harness":{"context":..,"harness":..}}`,
`{"item":{"context":..,"interface":..,"key":..}}`,
`{"clause":{"context":..,"contract":..,"label":..}}` and
`{"case":{"context":..,"harness":..,"name":..}}`. Every target has to exist when
the request opens, and a target is in one list, once (`cr.target-twice`,
`values.target`). `cr_open` refuses a field it
does not know (`change` and `remove` of earlier versions, and `crux`, among them).

- `targets`: what the request asks to have revised or retired. A request that
  adds something names what holds it, as adding is a move of the container: a
  Clause, its Contract; a Contract, Interface or Harness, its Context; a
  Context, the Project. A Context's revision moves when one of its assets is
  added or removed, or its `uses` or basis change, not when an asset's content
  does; the Project's moves when a Context is added or removed. A **parent**
  (a request other requests name as their `parent`) may have no targets: it keeps
  the request in the requester's words and the split, and is closed when its
  parts are.
- `keep`: the targets that must stay as they are; the revision recorded at
  `cr_open` is compared, whoever moved the target. A keep that an unresolved
  request's target breaks for sure is told when the two meet, not after the work:
  `cr.keep-targeted`, about the request that keeps, in the answer of the edit that
  made them meet (`cr_open`, `cr_crux_decide`, `cr_reopen`) and in `check`. It
  names the request that targets, the target and the target kept. Breaking for sure
  is the same target, or a Clause, Item or Case of a Contract, Interface or
  Harness that is kept (its revision moves with any of its parts). A kept Context
  is not told for a target inside it, nor a kept Item for the Interface that holds
  it; `cr.keep-moved` tells those after the work. To fix it, resolve the wrong request as
  `{"superseded":"<new title>"}` and open the new one with the right `keep` (the
  Item, not the whole Interface, when only its signature must stay).
- Each option of a Crux raised in the request takes the same `targets` / `keep` lists
  (and `wording`, the exact new text of the definitions, and `consequence`, free text
  saying what else choosing it affects). When a Crux is decided, the lists of the
  option chosen become the request's decided expectations, each with the revision it
  has at that moment (see "Crux, Policy and Assumption"). Deciding with the decision
  maker's own wording, leaving it unspecified, or withdrawing the question names
  nothing more.
- Neither list gates the closing of the request. `status` shows, for each of its
  `targets`, **what the work did to it**, read from the footprints of the
  request's change sets and of its parts (see "Attribution and frame"): `revised`,
  `retired` or `not touched yet`; and for each `keep`, whether it `moved`.
- `intent` (`change_behavior` or `keep_behavior`) is a record. No rule of
  resolving reads it; the note `cr.intent-mismatch` points out a keep-behavior
  request that names a Clause as a target, and a change-behavior request that
  names no target (a parent is left out).

### Lifecycle and resolution

```text
open ──cr_resolve──▶ resolved ──cr_reopen──▶ open ──cr_resolve──▶ resolved ...
```

`cr_resolve` is a decision, recorded with how the request ended, why, who says
so, and when (from the clock):

```json
{"op":"cr_resolve","title":"Refuse big discounts","resolution":"done",
 "reason":"C1 now refuses above 50%","by":{"person":"alice"}}
{"op":"cr_resolve","title":"Name the error","resolution":"wont_do",
 "reason":"callers do not tell the errors apart","by":{"role":"orchestrator"}}
{"op":"cr_resolve","title":"Cap discounts","resolution":{"duplicate":"Refuse big discounts"},
 "reason":"the same ask","by":{"person":"alice"}}
```

`resolution` is `"done"`, `"wont_do"`, `"invalid"`, `{"duplicate":"<title>"}` or
`{"superseded":"<title>"}`; the title is another change request, which has to
exist. The `reason` and `by` (`{"person":..}` or `{"role":..}`) are required.
Only explicit states refuse it, in this order:

1. the request is resolved already;
2. a Crux raised in it has no decision, whatever the resolution (`done`, `wont_do`,
   `invalid`, a duplicate or a superseded one: `cr.crux-unanswered`). The Cruxes
   raised in a request are decided or withdrawn before it ends;
3. a `duplicate` or a `superseded` points at itself or at a request that does not
   exist;
4. a request whose `parent` it is is not resolved (whatever way it would end).

The revisions of the targets and the work recorded for the request are **not**
consulted, so a request whose targets were never touched can be resolved as done.
The checks then say where the record and the work disagree (below).

`cr_reopen` opens a resolved request again:

```json
{"op":"cr_reopen","title":"Refuse big discounts","reason":"a caller still breaks",
 "by":{"person":"bob"}}
```

The resolution it had stays: a request keeps a **history**, a log of
`resolved` and `reopened` steps (each with the reason, who and when) that is only
appended to, and `status` shows it as `history` beside the current `resolution`.
A reopened request takes work (`change_set` `for` it), Cruxes and assumptions
again, and can be resolved anew. Reopening does not touch its parts or its
parent: a parent resolved while this request was closed stays resolved.

### Opening, and what `status` and the checks show

```json
{"op":"cr_open","title":"Refuse big discounts","intent":"change_behavior",
 "situation":"discounts above 50% should never be given","issuer":"sales",
 "targets":[{"clause":{"context":"pricing","contract":"discount","label":"C1"}}]}
```

```json
{"op":"cr_open","title":"Add rules and drop C2","intent":"change_behavior",
 "targets":[{"context":"pricing"},{"clause":{"context":"pricing","contract":"discount","label":"C2"}}],
 "keep":[{"clause":{"context":"pricing","contract":"discount","label":"C1"}}]}
```

A parent that only splits the ask, and a part with the targets (the shape of
removing a feature):

```json
{"op":"cr_open","title":"Remove release","intent":"change_behavior","situation":"release is gone"}
{"op":"cr_open","title":"Drop release","intent":"change_behavior","parent":"Remove release",
 "targets":[{"clause":{"context":"core","contract":"stock","label":"C7"}}]}
```

`cr_open` answers with what it recorded, so a wrong reference is seen at once:
`{"kind":"keep","target":{"clause":{"context":"pricing","contract":"discount","label":"C1"}},"recorded":1}`
(`kind` is `target` or `keep`). `status` shows each request with its `situation`,
`issuer`, `state`, `resolution` and `history`, the ids of the Cruxes raised in it
(`cruxes`), its `targets` with what the work did to each, its `keep` with whether
each moved, its `work` and its `out_of_frame` (the moves of its work outside its
frame, by the rule of "Attribution and frame": a Case that cites what the frame
holds is inside). It lists every Crux under `cruxes` (the
record `view` gives, with the request it was raised in by its title as `raised_in`),
each Crux that has no decision under `awaiting` (its id, `raised_in`, the question,
kind, options with what each names and its `consequence`, the recommendation with its
confidence, and the importance), and the unresolved requests one of whose `keep`
moved under `keep_moved`. The checks:

| check | level | says |
|---|---|---|
| `cr.awaiting-decision` | warning | a Crux has no decision, whether or not the request it was raised in is resolved |
| `cr.keep-moved` | warning | an unresolved request, or the one being resolved, keeps a target and it moved |
| `cr.keep-targeted` | warning | an unresolved request keeps a target that an unresolved request (itself included) names to revise or retire: the same target, or a Clause, Item or Case of the Contract, Interface or Harness it keeps |
| `cr.out-of-frame` | warning | the work for an unresolved request, or for the one being resolved, moved something it did not name (a Case that cites what it names is inside) |
| `cr.resolved-without-work` | warning | resolved as `done`, yet no change set `for` it or its parts moved anything |
| `cr.targets-untouched` | note | resolved as `done`, with work, yet a target it named was never touched by it or its parts |
| `cr.intent-mismatch` | note | the intent and the targets disagree |

`cr.unapplied` of earlier versions is gone: it judged the closing of a request by
the revisions of its targets, which a resolution no longer does; `status` shows
the same thing (`not touched yet`) per target, without a finding on every request
that has just been opened. A database written before targets and resolutions (or
before the Crux, Policy and Assumptions below, or before the work of a request was
recorded) does not load: there is no migration for it. A store of layout 1, which
held a Crux inside its change request, is migrated when it is opened (see "The
store's layout").

The change a request asks for is made as a `change_set` `for` it (see "Change
sets"): a change that touches several assets and holds only as a whole (a Clause
removed, the Cases that cited it gone from their Harness, an Item removed with it)
is one set, not a series of puts that has to be ordered around the citations.

### Attribution and frame

A `change_set` with `for` records its footprint for the request (what it moved, in
the same commit). That ties the work to the request that asked for it. A set that
moves nothing (the same puts applied again) is applied and answers `ok`, but it is
no work and records no footprint:

- What the work did to a `target` is read from the footprints recorded for the
  request or for its parts, children and their children (the requests that name
  it as their `parent`, and theirs). A target is `revised` when a footprint moved
  it, or something inside it, and it is still in the Project; `retired` when one
  moved it and it is gone; `not touched yet` when none did. A move in a set `for`
  another request, or in a set, a put or a retire without `for`, shows in no
  request's targets. This is information for `status` and the checks; it decides
  nothing about closing. A `keep` is a protection against anyone, so it is judged
  by the revision alone.
- A parent only aggregates. It need not name any target: the work of its parts is
  its work, and it is closed when they are resolved. A parent resolved as done
  with no work on it or on any part is `cr.resolved-without-work`.
- The targets a request names to revise or retire (its own and those of the
  options chosen for the Cruxes raised in it) are its **frame**; a `keep` is not part of it. Naming a
  container frames everything in it: a Contract frames its Clauses, a Context its
  assets and what is in them, the Project everything. The work of a request is to
  stay in its frame, and **the frame is the request's own**: a parent's targets
  neither widen nor bound its parts. A footprint that moved a target outside the
  frame of the request it was applied for is reported as `cr.out-of-frame` (a
  warning) for that request, by the put that did it, by `cr_resolve` when the
  request is resolved, and by `check` for as long as it lasts, and `status` lists
  it as `out_of_frame` of that request next to its `work` (each footprint's
  `store_revision` and the targets it moved). A put and a resolve judge the whole
  model, as every check of their moment does, so their findings also name the
  other open requests that are out of frame. A request with no targets has all its
  work out of frame. A container that moved only because something inside it moved
  within the frame (the Contract that took a Clause, the Context that took an asset)
  is not outside. A resolved request is a record: it takes no more work (until it
  is reopened) and `check` and `status` do not report it for its frame; the
  resolution itself is told (the checks of the `resolve` moment judge the request
  being resolved although it is resolved by then: `cr.out-of-frame`,
  `cr.keep-moved` and `cr.intent-mismatch`).
- **The frame includes the Cases that cite what it frames.** A Case that a change
  set moved (made, changed or retired) is inside the frame when it cites a Clause
  or an Item that is, or is inside, a target the frame names: revising a Clause
  makes the Cases that cite it stale, and bringing them up to it is the same work.
  One citation is enough: a Case that cites a framed Clause is inside though its
  other citations are not. The citations are those recorded in the footprint when
  the work was applied (a Case the set changed counts what it cited before the set
  and after it, so a set that retires a framed Clause and moves its Cases off it
  still has them inside; one that was made counts after, one that was retired
  counts before), so later work cannot move earlier work in or out of the frame. A
  footprint recorded before citations were kept is judged by containment alone.
  The Harness that moved only because such a Case moved is covered as any
  container is. For a move outside the frame: name the target when the request is
  opened, apply the change set `for` the request it belongs to, or take the change
  back.
- A new Clause or Item is a move of its own: a frame that names only the other
  Clauses of the Contract has it outside, so name the Contract when the request
  adds to it.
- **A new name moves nothing.** A Clause given another label, or an Item another key,
  keeps its id and its revision (and its Contract's or Interface's), so the footprint
  of the set that did it is empty: it is outside no frame, it breaks no `keep` of the
  Clause or its container, and no citation goes stale. A request whose only work is
  such a rename has no work: resolved as done, it is `cr.resolved-without-work`, and
  a Clause or Item it names as a target reads `not touched yet` (`cr.targets-untouched`
  when the request has other work).

### Crux, Policy and Assumption

Three things say how a question that came up while raising a request is settled,
by how it can be answered:

| | when | who | recorded |
|---|---|---|---|
| Assumption | there is no choice: the repository and general knowledge give the answer | the role at work | the statement, the reason, the evidence |
| Crux settled by the Policy | a choice must be made, and a line of the Policy derives it | a decision maker (a role such as `orchestrator`) | the choice, the Policy line applied and the Policy revision |
| Crux nobody derives | not even the Policy settles it | a person | the choice, the reason, who decided |

A **Crux** is a choice that cannot be derived, neither from the repository and
general knowledge (that is an Assumption) nor from the Project's decision Policy,
so a decision maker picks. It has no default: what depends on the answer waits,
and what does not goes on. Most choices are settled by the Policy and recorded as
such; what is left is an arbitrary, intentional decision, and the point is to
record who decided what, on which basis, strictly and immutably, because it
cannot be reproduced. It applies to every change request and finding (Contracts,
Interfaces, Harness implementation, code), not only to the specification.

- A Crux is a record of its own, with an `id`; a change request does not hold it.
  It is **raised in** a change request (`raised_in`), the one it stops: normally the
  request in whose work the question came up, but any request will do. A request can
  be stopped by several Cruxes, and a Crux stops one request. It may be raised from
  findings (`findings`; none for a question that arises from the text of the
  request). It has a `kind` (`ambiguity`: the text reads more than one way;
  `conflict`: two promises cannot both hold; `model`: the words of the model are in
  question; `disagreement`: a test and the promise it checks read differently), its
  `question`, its `options` (each with an `id`, the expectations it asks and,
  optionally, its `consequence`: what else choosing it affects), a recommended
  option with a `reason` and a `confidence` that is a percentage from 0 to 100, an
  `importance` (`low`, `normal` or `high`), and `supersedes`, the id of a Crux whose
  decision this one replaces (it has to exist and have a decision to replace, and a
  withdrawn one has none: `cr.supersedes-unknown`, `cr.supersedes-unanswered`). The
  recommendation and the importance are what a Policy line is written against.
- `cr_crux_raise` raises a Crux in the request titled `in`, which must not be
  resolved (`cr.already-resolved`: reopen it first):

  ```json
  {"op":"cr_crux_raise","in":"Refuse big discounts","question":"Which error is given?",
   "kind":"ambiguity","importance":"normal",
   "findings":[{"rule":"cite.uncited-clause","subject":{"clause":{"context":"pricing","contract":"discount","label":"C1"}}}],
   "options":[{"choice":"reuse InvalidDiscount","recommended":{"reason":"no new type","confidence":85}},
              {"choice":"new error","wording":"C2: a refused discount is DiscountRefused",
               "targets":[{"clause":{"context":"pricing","contract":"discount","label":"C2"}}],
               "keep":[{"clause":{"context":"pricing","contract":"discount","label":"C3"}}],
               "consequence":"callers match a new error"}]}
  ```

  `question`, `kind` and `options` are required; there is at least one option
  (`cr.crux-no-option`) and no choice is given twice (`cr.crux-option-twice`,
  `values.option`). At most one option has `recommended` (`cr.crux-recommended-twice`,
  `values.count`; its `reason` and `confidence` are optional). An option's `targets`
  and `keep` are written like the request's and have to exist when the Crux is
  raised. A target that an option names twice is refused (`cr.target-twice`,
  `values.target` and `values.option`), and one that the request, or a decision taken
  in it that stands, asks with the other kind is refused (`cr.crux-expect-conflict`,
  `values.option` and `values.target`). The result has
  the id of the Crux and of each option, which a decision points at:
  `{"op":"cr_crux_raise","crux":"<crux id>","options":[{"id":"<option id>","choice":"reuse InvalidDiscount"},..],"findings":[]}`.
  An id in angle brackets, here and below, is the one an earlier result gave.
- A **decision is required**, and "undefined" is a decision. `cr_crux_decide` gives it,
  once, with exactly one of `option` (the id of one of the Crux's options, with an
  optional `reason`), `wording` (the behaviour in the decision maker's own words, with
  an optional `reason`), `unspecified` (the behaviour is left free; a list of what must
  still hold, which may be empty) and `withdrawn` (`true`: the question is not needed
  any more), with the `basis` and `by`:

  ```json
  {"op":"cr_crux_decide","crux":"<crux id>","option":"<option id>","reason":"no new type",
   "basis":{"arbitrary":"no line of the Policy covers it"},"by":{"person":"alice"}}
  {"op":"cr_crux_decide","crux":"<crux id>","unspecified":["a refusal is an error"],
   "basis":{"policy":"P1"},"by":{"role":"orchestrator"}}
  {"op":"cr_crux_decide","crux":"<crux id>","withdrawn":true,
   "basis":{"arbitrary":"the clause it asked about is gone"},"by":{"person":"alice"}}
  ```

  The `basis` is `{"policy":"P1"}` (a line the Policy has now; the revision of the
  Policy is recorded with it) or `{"arbitrary":"<reason>"}`. A decision to leave it
  unspecified or to withdraw the question has no `reason` of its own: the basis has
  it. The time comes from the clock of the application, never from the domain. An
  answer is never rewritten: deciding a Crux that has one is refused
  (`cr.crux-answered`), and an option the Crux does not have is not one
  (`cr.crux-unknown-option`). A `crux` that no Crux has the id of is a name that does
  not exist (`input.unknown-name`, `values.kind` `crux`, with the ids that exist in
  `did_you_mean`). Changing a decision means raising a new Crux that
  `supersedes` the old one; the old answer stays as it was given. A Crux of kind
  `model` is the one exception to "undefined is an answer": without the word the
  model cannot be written, so it can be worded, chosen or withdrawn but not left
  unspecified (`cr.crux-unspecified-not-allowed`, `values.crux` and `values.kind`).
  A Crux that is no longer needed is withdrawn (any kind may be); a
  withdrawal is a decision and asks nothing of any target.
- **Who may decide** a Crux (the authority to decide, `CruxAnswerable`) is a concept
  of the model, and it is managed outside chidev for now: chidev holds no identity of
  its users, so it grants and checks nothing. `by` (`{"role":..}` or `{"person":..}`)
  is recorded as declared, and nothing is compared to it. The boundary is decided in
  [ADR 0003](docs/adr/0003-the-authority-to-decide-a-crux-is-managed-outside-chidev.md).
- The Cruxes stop the request they are raised in: "Lifecycle and resolution" says
  which resolutions are refused (`cr.crux-unanswered`), and a request with Cruxes is
  `decided` once each of them has a decision (a withdrawal is one). When a Crux is
  decided, the targets of the option chosen become the request's decided
  expectations, in the same commit: each is recorded with the revision it has then;
  one an earlier decision recorded with the same kind keeps its first revision, one
  the request itself asks with the same kind is not added a second time, and one asked
  with another kind, by the request or by an earlier decision, is refused
  (`cr.crux-expect-conflict`, the rule an option is read by when the Crux is raised).
  A Crux that
  supersedes a decision taken in the same request takes that decision's expectations
  back first, in the same commit, so it may ask the opposite of a target. They are
  part of the request's frame (see "Attribution and frame"), and the work goes `for`
  the request. A Crux of a resolved request takes only a withdrawal: any other
  decision is refused (`cr.already-resolved`) until the request is reopened. A store
  migrated from layout 1 can hold a Crux that was never decided in a request resolved
  otherwise than as `done`; it is a record of what was, and `cr.awaiting-decision`
  goes on reporting it.
- `cr_decide` and the `crux` of `cr_open` are gone; these two operations replace
  them. A line that has one is refused (`input.unknown-op`, `input.unknown-field`),
  and `did_you_mean` does not name the new one. Raise the question with
  `cr_crux_raise`, in the request it stops, where `crux` was given, and decide it
  with `cr_crux_decide` where `cr_decide` was written. The Crux is named by its id and
  not by the title of its request, and a decision names the option by its id
  (`option`) and not by the text of its choice (`choice`).
- The **Policy** is the Project's decision policy: numbered lines `P1`, `P2`, ...
  each with a text. `policy_put` replaces all of them:
  `{"op":"policy_put","lines":[{"id":"P1","text":"take the recommendation when its confidence is 80 or more"}]}`.
  Its revision goes up when the lines change, and `status.policy` shows the
  revision and the lines. The note `crux.policy-line-gone` reports an answer that
  rests on a line which is gone or whose text changed since (by the `policy_put`
  that did it, and by `check` for as long as it lasts); the answer itself stands.
- An **Assumption** `{statement, reason, evidence}` is an interpretation settled
  from the repository and general knowledge, so nobody is asked. `evidence` is a
  list of `{"repo":{"path":"src/round.rs","lines":[10,14]}}` (a path relative to
  the repository, without `..`; `lines` optional, the first and last line, as
  `[16,16]` for one line) and
  `{"knowledge":{"reference":"IEEE 754 roundTiesToEven"}}`. Give them with
  `cr_open.assumptions`, or append later with
  `{"op":"cr_assume","title":..,"statement":..,"reason":..,"evidence":[..]}`.
  Assumptions only grow, and a resolved request takes none. `status` lists them
  with each request.

## Retiring

A spec asset is not deleted, it is retired: it leaves the live model, and the
Project keeps a record so that older Pins, change requests and measurements can
still be read. Four operations, one per thing that holds others:
`{"op":"context_retire","context":..}`,
`{"op":"contract_retire","context":..,"contract":..}`,
`{"op":"interface_retire","context":..,"interface":..}` and
`{"op":"harness_retire","context":..,"harness":..}`. Clauses, Items and Cases
are removed as before, by putting their Contract, Interface or Harness again
without them; what a put drops is recorded as retired too.

- **The parts go with the whole.** A Context takes its Contracts, Interfaces and
  Harnesses with it, and an asset its Clauses, Items or Cases.
- **A peer that refers stops it.** Retiring is refused, naming what stands in the
  way, when a Case that stays cites an Item or a Clause that would go (retire or
  put the Harness that cites first), and when a Context is used by another one.
  A Context based on its Contracts keeps its last Contract.
- **Ids are reserved.** A retired id is never given again, and so a name an asset
  had in its Context cannot be put again (`interface_put` of a retired name is
  refused); make it again under another name. There is no way back.
- **The record keeps names and revisions, not the text**: for each thing its kind,
  names (the Context, the asset, and for a part the label, key or case
  name), the revision it had (`last_revision`) and `at`, the revision of what held
  it once it was retired (the Project's for a Context, the Context's for an asset
  and what went with it, the asset's for a part a put dropped). `status.retired`
  lists them, in the order they were retired.
- **Revisions.** The Project's revision moves when a Context is retired, a
  Context's when one of its assets is, so the Project or the Context named as a
  target reads `revised` as for adding.
- **Outside the Project, ids stay.** A Pin of a retired Interface stays and reads
  `gone` (`pin.stale` does not report it: retiring is intentional). A target that
  the work moved and that is retired reads `retired` in `status`. Measurements and
  findings keep their ids. Wherever chidev names a target the live model does not
  have it names it from the record with `(retired)` (`status`, errors, findings and
  `status.measurements`, whose `state` is then `retired`). The checks that judge
  Interfaces and Harnesses (`pin.unpinned`, `mutation.*`, `sim.*`, the uncited
  and no-harness checks) read the live model, so they do not judge a retired one.

Retiring, like a put, does not need an open change request, but a retire operation
belongs to none. The procedure retires as the work of a change request that asks
for the thing to go: open it with the Context or the asset in `targets`, retire it
in a `change_set` `for` the request (the Cases that cite first, or in the same
set), and resolve it as done once it has. `status` reads the target `retired` from
that work. A retire operation on its own is not tied to a request, and a thing it
takes out shows in no request's targets.

Retiring several things, or retiring together with a put, is one `change_set` with
`retires` (see "Change sets"): the refusals above are judged on the state the set
ends in, so a Harness can be put without the Cases that cite an Item and the
Interface retired in the same set. Retire operations on their own are change sets
of one, without `for`.

## Change-request addresses

Each entry in `cr_open.addresses`, and in the `findings` of `cr_crux_raise` (the
findings a Crux is raised from), names a rule and the subject of its finding. It
uses the same `subject` form as `finding_report`:

```json
[
  {"rule":"context.no-harness","subject":{"context":"pricing"}},
  {"rule":"pin.unpinned","subject":{"interface":{"context":"pricing","interface":"rounding"}}},
  {"rule":"cite.uncited-item","subject":{"item":{"context":"pricing","interface":"rounding","key":"round"}}},
  {"rule":"cite.uncited-clause","subject":{"clause":{"context":"pricing","contract":"totals","label":"C1"}}},
  {"rule":"judged.case-review","subject":{"case":{"context":"pricing","harness":"rounding-tests","name":"half rounds up"}}},
  {"rule":"cr.keep-moved","subject":{"change_request":"change rounding"}},
  {"rule":"judged.project-review","subject":null}
]
```

Omitting `subject` also names the whole Project. The legacy address forms with
an `item` or `clause` directly beside `rule` remain accepted. Each address may
contain only one selector key (`subject`, `item`, or `clause`), even when a value
is null. Unknown address fields and unknown fields inside a subject are rejected.

Names must resolve when the change request opens. After that, the reference uses
stable ids and survives later changes to the subject. The rule need not be
registered or currently report a finding; its id must be well formed. Opening a
request does not itself run a checker or judge a rule.

For Rust callers, `FindingInput` uses `subject: Option<SubjectInput>` in place of
its former `item` / `clause` fields. Wrap those references in
`SubjectInput::Item` / `SubjectInput::Clause`; `app::SubjectInput` remains the
public re-export. See `app::subject` and `domain::check::FindingState` in the
RustDoc for subject resolution and how a finding's state follows its requests.

## Diagnostics

A refused operation is a violation of a *rule*, and it is printed as structure, not
as a sentence to be parsed:

```json
{"ok":false,"line":9,"diagnostics":[{
  "code":"cr.parent-open","level":"error",
  "message":"the change request `Parent` has 2 part(s) that are not resolved: `Part one`, `Part two`",
  "subjects":[{"kind":"change_request","name":"Parent"},
              {"kind":"change_request","name":"Part one"},{"kind":"change_request","name":"Part two"}],
  "values":{"count":2},
  "help":"A part is a change request whose `parent` is this one. Resolve each part first ...",
  "suggestions":[{"op":{"op":"cr_resolve","title":"Part one","resolution":"done","reason":"<why>","by":{"person":"<who>"}},
                  "applicability":"placeholder"}],
  "did_you_mean":[]}]}
```

- `code` is the id of the rule and is stable: a published id keeps its meaning and
  is never reused, and the keys of a rule's `values` are added to, never removed.
  Read the `code` and the `values`; the `message` is for a person.
- `subjects` are what it is about, named as `status` shows them. `field` (when the
  refusal is about the input) says where: `for`, `puts[0].context`,
  `cases[0].items[0].interface`.
- `help` says why the rule holds and how to mend a violation. `suggestions` are the
  operations that mend it: `op` is an operation to send (with `"<..>"` where a value
  is yours to fill in), or a sentence when no single operation does it. Their
  `applicability` says how far to trust one, as rustc's does: `machine` (it mends the
  violation and changes nothing else: apply it as it stands), `review` (likely what
  is meant, but it changes the model: look first) and `placeholder` (it has a hole:
  fill it in). `did_you_mean` lists the names that exist and are close to one that
  does not (`input.unknown-name`): a title or a label one letter off, or the same in
  another case.
- A line that cannot be read as an operation is a diagnostic as well, under an
  `input.*` rule: `input.unknown-op`, `input.unknown-field`, `input.missing-field`,
  `input.wrong-type`, `input.unknown-name`, `input.invalid`, and for `view`
  `input.view-sections-empty`. An `interface_put` that
  leaves `items` out has its own: `input.items-required`, `items.no-provider`,
  `items.protocol`, `items.failed`, and for `renames` `input.rename-unknown`,
  `input.rename-conflict`, `input.replaces-kept`, `input.renames-with-items`. A
  `clauses_put`, or a `contract_put` with `replaces`, has `input.unknown-name` for a
  `replaces` or `remove` label the Contract does not have (`values.field` says which),
  `input.replaces-kept` and `input.rename-conflict` (`values.side`: `label`,
  `replaces` or `remove`). The subjects of the last two are the Context and the
  Contract; the subject of `input.unknown-name` is the Contract, as for an unknown
  cited label.
- A change refused by checks keeps its `findings` beside the diagnostics, and a
  finding in any result has the same fields (`code` is its rule, `help` its rule's).
  A refusal that has no rule of its own yet is `domain.unmapped` (or `app.unmapped`),
  which says what was refused; it is a bug to report.

A rule's `level` is what is shown, not what refuses. A change is refused for a
violation of an invariant (always `error`; no policy lowers it), for an `error` of a
*mandatory* check, and for a finding of a gate (see "External checks"); every other
check says what it finds at its level and the change is kept.

`{"op":"rules"}` lists every rule (the invariants an operation is refused for, the
input rules, and the checks with the level the policy gives them), and
`{"op":"explain","rule":"cr.parent-open"}` explains one: its help, the keys of the
values a diagnostic of it carries, and an operation that violates it. **An agent
that is refused should read `suggestions` and `help` before it tries again,** and run
`explain` for a rule it does not know.

In Rust, `ChangeRequestError`, `ProjectError`, `ContextError` and `PinError` give the
domain's `Violation` (`violation`, `violations`; see `domain::violation` and
`domain::rule`), and `App::diagnose` turns an `AppError` into `Vec<Diagnostic>`,
which any front end (the CLI, a server) lists as it is.

## Judged rule versions

When an agent reads `check_rules`, each rule includes a `rule_digest`. The agent
must return that exact value in its `finding_report`, including a report with no
findings. A missing, null or malformed digest is refused as an input error, and
one that differs from the current definition as a rule version mismatch; either
way the report replaces nothing and records no run. The result carries the saved
`rule_digest` and the freshness `state`.

The digest covers only the fields that decide the answer: `id`, `subject`,
`question`, `true` and `false` (see `JudgedRule::digest` in RustDoc). The
summary, fix, `when` hints, moments and default level describe, guide, schedule or
weight the rule without changing what is a finding, so editing them, TOML
comments or formatting, or a Policy level override leaves saved judgments
current. Changing a deciding field makes the saved report and every finding
stale. An App keeps its Registry for its lifetime; the CLI reloads
configuration on each invocation.

Database rows written before digests existed hold a null `rule_digest`: they
stay readable and show as stale (`rule_digest: null`), since the definition they
were judged on cannot be established after the fact. A matching digest still
cannot make a report on an older model current.

Rust callers supply `ReportInput.rule_digest` (a `String`) from
`RuleToJudge.rule_digest`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
