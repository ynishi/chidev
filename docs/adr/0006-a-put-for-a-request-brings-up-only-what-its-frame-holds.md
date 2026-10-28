# 6. A put for a request brings up only what its frame holds

Status: accepted

Builds on the frame of a change request (README, "Attribution and frame"): the
targets a request names to revise or retire, and what is inside them. This decision
is about which citations of a Harness put `for` a request are brought up to the
revision of what they cite.

## Context

A citation of a Case is recorded at a revision of its target. When the target is
revised later, the citation is stale until someone looks at the Case and puts it
again. Putting the Harness recorded every citation of every Case of it at the revision
its target has, whichever Case the put was about.

So a Harness put `for` part A of a request also brought up the Cases that part B's
design had made stale, and they moved: their recorded revisions changed. That was
wrong in two ways:

- The put claimed B's Cases current with B's new Clauses though nobody had looked at
  them, and B lost the stale signal that said they still had to be looked at.
- The Cases that moved were outside A's frame, so A was reported `cr.out-of-frame`
  for work that was not its own.

Parts that share a Harness therefore had to go one after the other: the design and
the Harness put of one part, then the next.

## Decision

**A change set `for` a request records each citation of each Case of a Harness it
puts at the revision its target has only where the request's frame holds it.**

- Per citation, not per Case. A citation is recorded at the revision its target has
  once the Contracts and Interfaces of the set are in, when
  - the Case is new (its id was not in the Harness before), or
  - the Case did not cite that target before, or
  - the target is inside the frame (it, or its Contract or Interface, its Context or
    the Project is named by the frame), or
  - the Case is inside the frame (the Case, its Harness, its Context or the Project
    is named).

  Otherwise it is recorded at the revision the Case had recorded for it before the
  set: a stale citation stays stale. A Case that cites a Clause of A and a stale
  Clause of B is brought up on the first by A's put and on the second by B's.
- A change set that belongs to no request (a put alone, or a set without `for`)
  records every citation at the revision its target has, as before.
- Whether the Case's code changed does not decide it. `Harness::revise_as` is
  unchanged: a Case whose citations, name and code are what they were keeps its
  revision, and one whose code changed moves.
- "Inside the frame" has one definition: the `Frame` value, which says whether a
  target is held by the targets a request names (it, or a container of it, is named)
  and whether a Case is (also through what it cites). The work a request did, what an
  answer adds to a request, and this rule all ask it. The change set carries the
  frame of the request it is applied for; the Project works out only the containers
  of a citation and of the Case, and holds no rule of its own about what is inside.

## Consequences

- Parts that share a Harness are designed side by side, each part's Harness put
  moving only the Cases its frame holds and the Cases whose code it changes. A stale Case of another part stays stale and
  does not move, and `status` still lists it.
- A Case whose code a put changes still moves, in any put. What goes side by side is
  the design of the parts and the refresh of their citations, not two writers editing
  one Case.
- The edge: a set `for` a request that revises a Clause outside its frame and puts a
  Harness whose unchanged Case cites it leaves that citation at the revision it had,
  so the Case is stale right after the set. The revision itself is `cr.out-of-frame`.
  The work outside the frame is not made to look finished.
- A plain `harness_put` (and any put without `for`) is unchanged: it brings up every
  citation.
- A footprint of a put records only moves, as before; there is no kind of move for a
  refresh. A refresh inside the frame is the move of a Case that is inside the frame,
  and outside the frame nothing moves.

## Alternatives considered

- A footprint kind of "refresh" that the frame does not count. It hides the
  `cr.out-of-frame` warning but the put still claims the other part's Cases current,
  and removes their stale signal.
- A put that refreshes only the Cases it names. It changes every existing flow (a
  re-put of the Harness clears `stale` today) and needs a new input to say which.
- Keep the procedure of one part after the other. It leaves the model saying that a
  put claims what it did not look at, and the work of one part blocked on another's.
