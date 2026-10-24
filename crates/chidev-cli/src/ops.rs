//! The operations `chidev exec` reads, one JSON object per line, and the results it prints. This is where the JSON
//! form is turned into the inputs of the use cases and back; nothing past it sees JSON.

use std::io::{self, BufRead, Write};

use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, CruxInput, CruxKind,
    DecisionInput, FindingInput, Found, Intent, ItemInput, ItemRef, OptionInput, ParamValue,
    PinState, ReasonedInput,
};
use serde::{Deserialize, Serialize};
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
    /// Opens a change request.
    CrOpen(Box<OpenDto>),
    /// Decides the Crux of a change request: exactly one of `choice`, `wording` and `unspecified`.
    CrDecide {
        title: String,
        #[serde(default)]
        choice: Option<String>,
        #[serde(default)]
        wording: Option<String>,
        #[serde(default)]
        unspecified: Option<Vec<String>>,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Closes the record of a change request whose change is applied.
    CrResolve { title: String },
    /// Where the project stands.
    Status,
    /// Runs every check on the project as it is; nothing is kept.
    Check,
    /// Lists every check with the level it runs at.
    CheckList,
}

/// The fields of `cr_open`, boxed: they are far larger than any other operation's.
#[derive(Debug, Deserialize)]
struct OpenDto {
    title: String,
    #[serde(default)]
    situation: Option<String>,
    intent: IntentDto,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    items: Vec<ItemRefDto>,
    #[serde(default)]
    clauses: Vec<ClauseRefDto>,
    /// The question for a person; none: the machine resolves it.
    #[serde(default)]
    crux: Option<CruxDto>,
    /// The findings it deals with.
    #[serde(default)]
    addresses: Vec<AddressDto>,
}

/// A finding a change request addresses: the rule, and the item or clause it is about.
#[derive(Debug, Deserialize)]
struct AddressDto {
    rule: String,
    #[serde(default)]
    item: Option<ItemRefDto>,
    #[serde(default)]
    clause: Option<ClauseRefDto>,
}

impl From<AddressDto> for FindingInput {
    fn from(a: AddressDto) -> FindingInput {
        FindingInput {
            rule: a.rule,
            item: a.item.map(ItemRef::from),
            clause: a.clause.map(ClauseRef::from),
        }
    }
}

/// A finding in a result.
#[derive(Debug, Serialize)]
struct FindingOut<'a> {
    rule: &'a str,
    level: &'static str,
    /// `open`, `addressed` or `decided`.
    state: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    about: Option<&'a str>,
}

fn findings_out(found: &[Found]) -> Vec<FindingOut<'_>> {
    found
        .iter()
        .map(|f| FindingOut {
            rule: f.rule.as_str(),
            level: f.level.word(),
            state: f.state.word(),
            message: &f.message,
            about: f.about.as_deref(),
        })
        .collect()
}

/// The result of `check`.
#[derive(Debug, Serialize)]
struct CheckOut<'a> {
    op: &'static str,
    findings: Vec<FindingOut<'a>>,
}

/// A parameter's value in a listing.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ParamOut {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// A check in the result of `check_list`.
#[derive(Debug, Serialize)]
struct CheckEntryOut {
    id: String,
    summary: &'static str,
    level: &'static str,
    default_level: &'static str,
    mandatory: bool,
    executor: &'static str,
    hooks: Vec<&'static str>,
    params: std::collections::BTreeMap<String, ParamOut>,
}

/// The result of `check_list`: the checks, and a fingerprint of the policy they run under.
#[derive(Debug, Serialize)]
struct CheckListOut {
    op: &'static str,
    policy: String,
    checks: Vec<CheckEntryOut>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntentDto {
    ChangeBehavior,
    KeepBehavior,
}

impl From<IntentDto> for Intent {
    fn from(i: IntentDto) -> Intent {
        match i {
            IntentDto::ChangeBehavior => Intent::ChangeBehavior,
            IntentDto::KeepBehavior => Intent::KeepBehavior,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CruxDto {
    kind: KindDto,
    options: Vec<OptionDto>,
    #[serde(default)]
    default: Option<ReasonedDto>,
    #[serde(default)]
    recommended: Option<ReasonedDto>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindDto {
    Ambiguity,
    Conflict,
    Model,
    Disagreement,
}

impl From<KindDto> for CruxKind {
    fn from(k: KindDto) -> CruxKind {
        match k {
            KindDto::Ambiguity => CruxKind::Ambiguity,
            KindDto::Conflict => CruxKind::Conflict,
            KindDto::Model => CruxKind::Model,
            KindDto::Disagreement => CruxKind::Disagreement,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OptionDto {
    choice: String,
    #[serde(default)]
    change: Option<String>,
    #[serde(default)]
    consequence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReasonedDto {
    choice: String,
    #[serde(default)]
    reason: Option<String>,
}

impl From<ReasonedDto> for ReasonedInput {
    fn from(r: ReasonedDto) -> ReasonedInput {
        ReasonedInput {
            choice: r.choice,
            reason: r.reason,
        }
    }
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
pub async fn run<S, G>(
    app: &mut App<S, G>,
    input: impl BufRead,
    out: &mut impl Write,
) -> io::Result<bool>
where
    S: Store,
    G: IdSource,
{
    let mut all_ok = true;
    for (k, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let result = match serde_json::from_str::<Op>(&line) {
            Ok(op) => apply(app, op).await,
            Err(e) => Err(AppError::Input(format!("not an operation: {e}"))),
        };
        let mut result = match result {
            Ok(Value::Object(mut fields)) => {
                fields.insert("ok".to_string(), json!(true));
                Value::Object(fields)
            }
            Ok(other) => json!({ "ok": true, "result": other }),
            Err(AppError::Refused(found)) => {
                all_ok = false;
                json!({ "ok": false, "error": AppError::Refused(found.clone()).to_string(), "findings": findings_out(&found) })
            }
            Err(error) => {
                all_ok = false;
                json!({ "ok": false, "error": error.to_string() })
            }
        };
        result["line"] = json!(k + 1);
        writeln!(out, "{result}")?;
    }
    Ok(all_ok)
}

/// A typed result as the JSON object the run prints.
fn to_value(out: &impl Serialize) -> Result<Value, AppError> {
    serde_json::to_value(out).map_err(|e| AppError::Input(format!("cannot write the result: {e}")))
}

async fn apply<S, G>(app: &mut App<S, G>, op: Op) -> Result<Value, AppError>
where
    S: Store,
    G: IdSource,
{
    Ok(match op {
        Op::Init => json!({ "op": "init", "project": app.init().await?.to_string() }),
        Op::ContextAdd { name, uses } => {
            json!({ "op": "context_add", "context": app.add_context(&name, &uses).await?.to_string() })
        }
        Op::ContextUses { context, uses } => {
            app.set_uses(&context, &uses).await?;
            json!({ "op": "context_uses" })
        }
        Op::BaseOnContract { context } => {
            app.base_on_contract(&context).await?;
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
            let done = app.put_contract(&context, &name, &title, &clauses).await?;
            json!({ "op": "contract_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
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
            let done = app.put_interface(&context, &name, &source, &items).await?;
            json!({ "op": "interface_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
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
            let done = app.put_harness(&context, &name, &source, &cases).await?;
            json!({ "op": "harness_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::Pin {
            context,
            interface,
            code,
        } => {
            let done = app.pin(&context, &interface, &code).await?;
            json!({ "op": "pin", "pin": done.value.to_string(), "findings": findings_out(&done.findings) })
        }
        Op::CrOpen(open) => {
            let OpenDto {
                title,
                situation,
                intent,
                parent,
                issuer,
                items,
                clauses,
                crux,
                addresses,
            } = *open;
            let crux = crux.map(|q| CruxInput {
                kind: q.kind.into(),
                options: q
                    .options
                    .into_iter()
                    .map(|o| OptionInput {
                        choice: o.choice,
                        change: o.change,
                        consequence: o.consequence,
                    })
                    .collect(),
                default: q.default.map(ReasonedInput::from),
                recommended: q.recommended.map(ReasonedInput::from),
            });
            let input = ChangeRequestInput {
                title,
                situation,
                intent: intent.into(),
                parent,
                issuer,
                items: items.into_iter().map(ItemRef::from).collect(),
                clauses: clauses.into_iter().map(ClauseRef::from).collect(),
                crux,
                addresses: addresses.into_iter().map(FindingInput::from).collect(),
            };
            let id = app.open_change_request(&input).await?;
            json!({ "op": "cr_open", "cr": id.to_string() })
        }
        Op::CrDecide {
            title,
            choice,
            wording,
            unspecified,
            reason,
        } => {
            let decision = match (choice, wording, unspecified) {
                (Some(choice), None, None) => {
                    DecisionInput::Chosen(ReasonedInput { choice, reason })
                }
                (None, Some(wording), None) => DecisionInput::Worded { wording, reason },
                (None, None, Some(keep)) => DecisionInput::Unspecified { keep },
                _ => {
                    return Err(AppError::Input(
                        "give exactly one of `choice`, `wording` and `unspecified`".to_string(),
                    ));
                }
            };
            app.decide_change_request(&title, &decision).await?;
            json!({ "op": "cr_decide" })
        }
        Op::CrResolve { title } => {
            let done = app.resolve_change_request(&title).await?;
            json!({ "op": "cr_resolve", "findings": findings_out(&done.findings) })
        }
        Op::Status => {
            let s = app.status().await?;
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
            let crs: Vec<Value> = s
                .change_requests
                .iter()
                .map(|c| {
                    json!({
                        "title": c.title, "intent": c.intent, "parent": c.parent, "issuer": c.issuer,
                        "crux": c.crux, "state": c.state, "decision": c.decision,
                    })
                })
                .collect();
            let open: Vec<Value> = s
                .unapplied
                .iter()
                .map(|u| json!({ "cr": u.cr, "decision": u.decision, "unchanged": u.unchanged, "gone": u.gone }))
                .collect();
            json!({ "op": "status", "pins": pins, "stale_cites": stale, "change_requests": crs, "unapplied": open, "findings": findings_out(&s.findings) })
        }
        Op::Check => {
            let found = app.check().await?;
            to_value(&CheckOut {
                op: "check",
                findings: findings_out(&found),
            })?
        }
        Op::CheckList => {
            let registry = app.registry();
            let checks = registry
                .list()
                .into_iter()
                .map(|c| CheckEntryOut {
                    id: c.descriptor.id.to_string(),
                    summary: c.descriptor.summary,
                    level: c.level.word(),
                    default_level: c.descriptor.default_level.word(),
                    mandatory: c.descriptor.mandatory,
                    executor: c.descriptor.executor.word(),
                    hooks: c.descriptor.hooks.iter().map(|h| h.word()).collect(),
                    params: c
                        .params
                        .iter()
                        .map(|(name, v)| {
                            let out = match v {
                                ParamValue::Bool(b) => ParamOut::Bool(*b),
                                ParamValue::Int(i) => ParamOut::Int(*i),
                                ParamValue::Str(s) => ParamOut::Str(s.clone()),
                            };
                            (name.to_string(), out)
                        })
                        .collect(),
                })
                .collect();
            to_value(&CheckListOut {
                op: "check_list",
                policy: registry.policy().hash().to_string(),
                checks,
            })?
        }
    })
}
