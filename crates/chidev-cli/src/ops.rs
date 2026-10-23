//! The operations `chidev exec` reads, one JSON object per line, and the results it prints. This is where the JSON
//! form is turned into the inputs of the use cases and back; nothing past it sees JSON.

use std::io::{self, BufRead, Write};

use chidev_core::app::ports::{AssetNames, IdSource};
use chidev_core::app::{
    App, AppError, CaseInput, ClauseInput, ClauseRef, ItemInput, ItemRef, PinState,
};
use chidev_core::domain::repository::{CruxRepository, PinRepository, ProjectRepository};
use serde::Deserialize;
use serde_json::{Value, json};

/// One operation. `op` names it; the other fields are its arguments.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    /// Makes the project.
    Init,
    /// Adds a Context.
    ContextAdd {
        name: String,
        #[serde(default)]
        uses: Vec<String>,
    },
    /// Sets the Contexts a Context uses.
    ContextUses { context: String, uses: Vec<String> },
    /// Bases a Context on its Contracts.
    BaseOnContract { context: String },
    /// Makes or revises a Contract.
    ContractPut {
        context: String,
        name: String,
        title: String,
        clauses: Vec<ClauseDto>,
    },
    /// Makes or revises an Interface.
    InterfacePut {
        context: String,
        name: String,
        source: String,
        items: Vec<ItemDto>,
    },
    /// Makes or revises a Harness.
    HarnessPut {
        context: String,
        name: String,
        source: String,
        cases: Vec<CaseDto>,
    },
    /// Pins the code of an Interface.
    Pin {
        context: String,
        interface: String,
        code: String,
    },
    /// Raises a Crux.
    CruxRaise {
        title: String,
        choices: Vec<String>,
        default: String,
        #[serde(default)]
        items: Vec<ItemRefDto>,
        #[serde(default)]
        clauses: Vec<ClauseRefDto>,
    },
    /// Answers a Crux.
    CruxAnswer { title: String, choice: String },
    /// Where the project stands.
    Status,
}

#[derive(Debug, Deserialize)]
struct ClauseDto {
    label: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ItemDto {
    signature: String,
    lines: [usize; 2],
    #[serde(default)]
    replaces: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ItemRefDto {
    #[serde(default)]
    context: Option<String>,
    interface: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct ClauseRefDto {
    #[serde(default)]
    context: Option<String>,
    contract: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct CaseDto {
    name: String,
    lines: [usize; 2],
    #[serde(default)]
    items: Vec<ItemRefDto>,
    #[serde(default)]
    clauses: Vec<ClauseRefDto>,
}

impl From<ItemRefDto> for ItemRef {
    fn from(d: ItemRefDto) -> ItemRef {
        ItemRef {
            context: d.context,
            interface: d.interface,
            signature: d.signature,
        }
    }
}

impl From<ClauseRefDto> for ClauseRef {
    fn from(d: ClauseRefDto) -> ClauseRef {
        ClauseRef {
            context: d.context,
            contract: d.contract,
            label: d.label,
        }
    }
}

/// Runs every line of `input` against `app` and writes one result line per operation to `out`. A blank line is
/// skipped. A line that fails (bad JSON or a refused operation) gets a result with `"ok": false` and the run goes
/// on. Returns whether every operation succeeded; an error is only a failure to read or write.
pub fn run<S, G>(app: &mut App<S, G>, input: impl BufRead, out: &mut impl Write) -> io::Result<bool>
where
    S: ProjectRepository + PinRepository + CruxRepository + AssetNames,
    G: IdSource,
{
    let mut all_ok = true;
    for (k, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let result = match serde_json::from_str::<Op>(&line) {
            Ok(op) => apply(app, op).map_err(|e| e.to_string()),
            Err(e) => Err(format!("not an operation: {e}")),
        };
        let mut result = match result {
            Ok(Value::Object(mut fields)) => {
                fields.insert("ok".to_string(), json!(true));
                Value::Object(fields)
            }
            Ok(other) => json!({ "ok": true, "result": other }),
            Err(error) => {
                all_ok = false;
                json!({ "ok": false, "error": error })
            }
        };
        result["line"] = json!(k + 1);
        writeln!(out, "{result}")?;
    }
    Ok(all_ok)
}

fn apply<S, G>(app: &mut App<S, G>, op: Op) -> Result<Value, AppError>
where
    S: ProjectRepository + PinRepository + CruxRepository + AssetNames,
    G: IdSource,
{
    Ok(match op {
        Op::Init => json!({ "op": "init", "project": app.init()?.to_string() }),
        Op::ContextAdd { name, uses } => {
            json!({ "op": "context_add", "context": app.add_context(&name, &uses)?.to_string() })
        }
        Op::ContextUses { context, uses } => {
            app.set_uses(&context, &uses)?;
            json!({ "op": "context_uses" })
        }
        Op::BaseOnContract { context } => {
            app.base_on_contract(&context)?;
            json!({ "op": "base_on_contract" })
        }
        Op::ContractPut {
            context,
            name,
            title,
            clauses,
        } => {
            let clauses: Vec<ClauseInput> = clauses
                .into_iter()
                .map(|c| ClauseInput {
                    label: c.label,
                    text: c.text,
                })
                .collect();
            let put = app.put_contract(&context, &name, &title, &clauses)?;
            json!({ "op": "contract_put", "id": put.id.to_string(), "revision": put.revision.number() })
        }
        Op::InterfacePut {
            context,
            name,
            source,
            items,
        } => {
            let items: Vec<ItemInput> = items
                .into_iter()
                .map(|i| ItemInput {
                    signature: i.signature,
                    lines: (i.lines[0], i.lines[1]),
                    replaces: i.replaces,
                })
                .collect();
            let put = app.put_interface(&context, &name, &source, &items)?;
            json!({ "op": "interface_put", "id": put.id.to_string(), "revision": put.revision.number() })
        }
        Op::HarnessPut {
            context,
            name,
            source,
            cases,
        } => {
            let cases: Vec<CaseInput> = cases
                .into_iter()
                .map(|c| CaseInput {
                    name: c.name,
                    lines: (c.lines[0], c.lines[1]),
                    items: c.items.into_iter().map(ItemRef::from).collect(),
                    clauses: c.clauses.into_iter().map(ClauseRef::from).collect(),
                })
                .collect();
            let put = app.put_harness(&context, &name, &source, &cases)?;
            json!({ "op": "harness_put", "id": put.id.to_string(), "revision": put.revision.number() })
        }
        Op::Pin {
            context,
            interface,
            code,
        } => {
            json!({ "op": "pin", "pin": app.pin(&context, &interface, &code)?.to_string() })
        }
        Op::CruxRaise {
            title,
            choices,
            default,
            items,
            clauses,
        } => {
            let items: Vec<ItemRef> = items.into_iter().map(ItemRef::from).collect();
            let clauses: Vec<ClauseRef> = clauses.into_iter().map(ClauseRef::from).collect();
            let id = app.raise_crux(&title, &choices, &default, &items, &clauses)?;
            json!({ "op": "crux_raise", "crux": id.to_string() })
        }
        Op::CruxAnswer { title, choice } => {
            app.answer_crux(&title, &choice)?;
            json!({ "op": "crux_answer" })
        }
        Op::Status => {
            let s = app.status()?;
            let pins: Vec<Value> = s
                .pins
                .iter()
                .map(|p| {
                    let (state, why) = match &p.state {
                        PinState::Unpinned => ("unpinned", Vec::new()),
                        PinState::Fresh => ("fresh", Vec::new()),
                        PinState::Gone => ("gone", Vec::new()),
                        PinState::Stale(why) => ("stale", why.clone()),
                    };
                    json!({ "context": p.context, "interface": p.interface, "state": state, "why": why })
                })
                .collect();
            let stale: Vec<Value> = s
                .stale_cites
                .iter()
                .map(|c| json!({ "case": c.case, "target": c.target, "cited": c.cited, "now": c.now }))
                .collect();
            let open: Vec<Value> = s
                .unapplied
                .iter()
                .map(|u| json!({ "crux": u.crux, "answer": u.answer, "unchanged": u.unchanged, "gone": u.gone }))
                .collect();
            json!({ "op": "status", "pins": pins, "stale_cites": stale, "unapplied": open })
        }
    })
}
