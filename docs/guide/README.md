# Guide

How a person or a coding agent works with chidev. The guide is not part of
chidev: chidev keeps the model and gives its data and the schema of that data,
and how anyone is told to use it lives outside it, here. Nothing in this
directory is read by the command or shipped with it.

## Reading order

1. [PROCEDURE.md](PROCEDURE.md): the roles, the loop from a request to pinned
   code, and what is never done.
2. The [README](../../README.md) for the model and every operation. The guide
   names the sections it relies on and does not repeat them.
3. The command itself, for what is exact today:
   - `chidev schema ops` and `chidev schema view`: the JSON Schema of a line of
     `exec` and of what `view` returns;
   - `{"op":"rules"}` and `{"op":"explain",..}`: every rule a result can name
     in its `diagnostics`, and what to do about one (see
     [Diagnostics](../../README.md#diagnostics));
   - `{"op":"view",..}` and `{"op":"status"}`: the model as it is kept, and
     the state of its Pins, change requests and checks;
   - `{"op":"check"}`, and `{"op":"check","context":..}` for one Context's
     share: what the whole model is found to lack. An edit does not say it, since
     an edit cannot finish the model; `{"op":"check_list"}` shows, for each
     check, the `kind` and the `moments` it runs at (see
     [Triggers and moments](../../README.md#triggers-and-moments)).

When this guide and the README or the RustDoc disagree, they win: the guide
describes a way of working, they describe what the command does.
