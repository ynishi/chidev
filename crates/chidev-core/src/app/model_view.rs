//! The view of the model an agent judges from: the Contexts with what they use and are based on, the Contracts with
//! their Clauses, the Interfaces with their source and Items, the Harnesses with their source and Cases, and the
//! change requests. The types are the output (typed and versioned, so that what a reader may rely on is written
//! once, here, and a later protocol for programs can send the same view): they are serialized as they are.
//!
//! The same entries decide whether a judged finding still holds. A finding is stamped with the fingerprint of the
//! entry of its subject when it is reported, and is stale once the entry is not what it was, or is gone. One
//! build serves both, so what the agent saw and what makes its finding stale cannot differ.

use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::project::Project;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{Subject, SubjectKind};

/// The version of the view. It is its own: the stored documents have a schema of their own, and the two change
/// apart. A change that removes or renames a field is a new version; a field added is not. (Version 2: a change
/// request's `lines` became `expects`, each with a kind.)
pub const VIEW_VERSION: u32 = 2;

/// The model as an agent is shown it, read when the store stood at `store_revision`.
#[derive(Debug, Clone, Serialize)]
pub struct ModelView {
    version: u32,
    store_revision: u64,
    contexts: Vec<ContextView>,
    change_requests: Vec<ChangeRequestView>,
}

/// A Context, with its assets.
#[derive(Debug, Clone, Serialize)]
pub struct ContextView {
    id: String,
    name: String,
    /// `interface` or `contract`.
    basis: &'static str,
    /// The names of the Contexts it uses.
    uses: Vec<String>,
    contracts: Vec<ContractView>,
    interfaces: Vec<InterfaceView>,
    harnesses: Vec<HarnessView>,
}

/// A Contract and its Clauses.
#[derive(Debug, Clone, Serialize)]
pub struct ContractView {
    id: String,
    name: String,
    title: String,
    revision: u64,
    clauses: Vec<ClauseView>,
}

/// A Clause.
#[derive(Debug, Clone, Serialize)]
pub struct ClauseView {
    id: String,
    label: String,
    text: String,
    revision: u64,
}

/// An Interface, its source and its Items.
#[derive(Debug, Clone, Serialize)]
pub struct InterfaceView {
    id: String,
    name: String,
    revision: u64,
    source: String,
    items: Vec<ItemView>,
}

/// An Item.
#[derive(Debug, Clone, Serialize)]
pub struct ItemView {
    id: String,
    signature: String,
    /// The first and last line in the source.
    lines: [usize; 2],
    revision: u64,
}

/// A Harness, its source and its Cases.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessView {
    id: String,
    name: String,
    revision: u64,
    source: String,
    cases: Vec<CaseView>,
}

/// A Case.
#[derive(Debug, Clone, Serialize)]
pub struct CaseView {
    id: String,
    name: String,
    lines: [usize; 2],
    revision: u64,
    cites: CitesView,
}

/// What a Case cites, each with the revision it recorded.
#[derive(Debug, Clone, Serialize)]
pub struct CitesView {
    items: Vec<CiteView>,
    clauses: Vec<CiteView>,
}

/// One citation.
#[derive(Debug, Clone, Serialize)]
pub struct CiteView {
    id: String,
    revision: u64,
}

/// A change request.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeRequestView {
    id: String,
    title: String,
    /// `change_behavior` or `keep_behavior`.
    intent: &'static str,
    /// `open`, `decided` or `resolved`.
    state: &'static str,
    /// What it expects of targets, each with the revision the target had when the request was opened.
    expects: Vec<ExpectView>,
    /// The findings it deals with.
    addresses: Vec<AddressView>,
}

/// What a change request expects of a target.
#[derive(Debug, Clone, Serialize)]
pub struct ExpectView {
    /// The kind of the target: `project`, `context`, `contract`, `interface`, `harness`, `clause`, `item` or
    /// `case`.
    target: &'static str,
    /// Its id; none for the Project.
    id: Option<String>,
    /// `change`, `remove` or `keep`.
    kind: &'static str,
    /// The revision the target had when the expectation was recorded.
    recorded: u64,
}

/// A finding a change request addresses.
#[derive(Debug, Clone, Serialize)]
pub struct AddressView {
    rule: String,
    /// `kind` and `id` of what it is about; none: the Project.
    subject: Option<SubjectView>,
}

/// What a finding is about, by id.
#[derive(Debug, Clone, Serialize)]
pub struct SubjectView {
    kind: &'static str,
    id: String,
}

/// The fingerprint of anything the view holds. Serializing these types cannot fail; were it to, the fingerprint of
/// nothing would make every entry look unchanged, which is why there is no way for it to.
fn stamp<T: Serialize + ?Sized>(entry: &T) -> Digest {
    Digest::of(&serde_json::to_vec(entry).unwrap_or_default())
}

impl ContextView {
    /// What stamps a Context: the Contexts' assets are stamped by their own entries.
    fn head(&self) -> (&str, &str, &str, &[String]) {
        (&self.id, &self.name, self.basis, &self.uses)
    }
}

impl ModelView {
    /// The view of `project` and `crs`, read when the store stood at `store_revision`. `names` are the names a
    /// person gave the assets, by id; an asset with none is shown by its id.
    pub fn build(
        store_revision: u64,
        project: &Project,
        crs: &[ChangeRequest],
        names: &BTreeMap<Uuid, String>,
    ) -> ModelView {
        let name = |id: Uuid| names.get(&id).cloned().unwrap_or_else(|| id.to_string());
        let contexts = project
            .contexts()
            .map(|c| ContextView {
                id: c.id().to_string(),
                name: c.name().to_string(),
                basis: c.basis().word(),
                uses: c
                    .uses()
                    .iter()
                    .map(|u| {
                        project
                            .context(*u)
                            .map_or_else(|| u.to_string(), |x| x.name().to_string())
                    })
                    .collect(),
                contracts: c
                    .contracts()
                    .map(|k| ContractView {
                        id: k.id().to_string(),
                        name: name(k.id().as_uuid()),
                        title: k.title().to_string(),
                        revision: k.revision().number(),
                        clauses: k
                            .clauses()
                            .iter()
                            .map(|cl| ClauseView {
                                id: cl.id().to_string(),
                                label: cl.content().label().to_string(),
                                text: cl.content().text().as_str().to_string(),
                                revision: cl.revision().number(),
                            })
                            .collect(),
                    })
                    .collect(),
                interfaces: c
                    .interfaces()
                    .map(|i| InterfaceView {
                        id: i.id().to_string(),
                        name: name(i.id().as_uuid()),
                        revision: i.revision().number(),
                        source: i.source().as_str().to_string(),
                        items: i
                            .items()
                            .iter()
                            .map(|it| ItemView {
                                id: it.id().to_string(),
                                signature: it.signature().to_string(),
                                lines: [it.lines().first(), it.lines().last()],
                                revision: it.revision().number(),
                            })
                            .collect(),
                    })
                    .collect(),
                harnesses: c
                    .harnesses()
                    .map(|h| HarnessView {
                        id: h.id().to_string(),
                        name: name(h.id().as_uuid()),
                        revision: h.revision().number(),
                        source: h.source().as_str().to_string(),
                        cases: h
                            .cases()
                            .iter()
                            .map(|k| CaseView {
                                id: k.id().to_string(),
                                name: k.name().to_string(),
                                lines: [k.lines().first(), k.lines().last()],
                                revision: k.revision().number(),
                                cites: CitesView {
                                    items: k
                                        .cites()
                                        .items()
                                        .iter()
                                        .map(|(id, r)| CiteView {
                                            id: id.to_string(),
                                            revision: r.number(),
                                        })
                                        .collect(),
                                    clauses: k
                                        .cites()
                                        .clauses()
                                        .iter()
                                        .map(|(id, r)| CiteView {
                                            id: id.to_string(),
                                            revision: r.number(),
                                        })
                                        .collect(),
                                },
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let change_requests = crs
            .iter()
            .map(|c| ChangeRequestView {
                id: c.id().to_string(),
                title: c.title().to_string(),
                intent: c.intent().word(),
                state: c.state().word(),
                expects: c
                    .expects()
                    .iter()
                    .map(|(t, e)| ExpectView {
                        target: t.word(),
                        id: t.as_uuid().map(|u| u.to_string()),
                        kind: e.kind.word(),
                        recorded: e.recorded.number(),
                    })
                    .collect(),
                addresses: c
                    .addresses()
                    .iter()
                    .map(|a| AddressView {
                        rule: a.rule().to_string(),
                        subject: a.subject().map(|s| SubjectView {
                            kind: s.kind().word(),
                            id: s.as_uuid().to_string(),
                        }),
                    })
                    .collect(),
            })
            .collect();
        ModelView {
            version: VIEW_VERSION,
            store_revision,
            contexts,
            change_requests,
        }
    }

    /// The revision of the store the view was read at.
    pub fn store_revision(&self) -> u64 {
        self.store_revision
    }

    /// The fingerprint of the entry of `subject` (of the whole model for a finding about the Project); none when
    /// the entry is gone.
    pub fn stamp_of(&self, subject: Option<Subject>) -> Option<Digest> {
        let Some(subject) = subject else {
            return Some(self.scope_stamp(SubjectKind::Project));
        };
        let id = subject.as_uuid().to_string();
        match subject {
            Subject::Context(_) => self
                .contexts
                .iter()
                .find(|c| c.id == id)
                .map(|c| stamp(&c.head())),
            Subject::Interface(_) => self.interfaces().find(|i| i.id == id).map(stamp),
            Subject::Item(_) => self.items().find(|i| i.id == id).map(stamp),
            Subject::Clause(_) => self.clauses().find(|c| c.id == id).map(stamp),
            Subject::Case(_) => self.cases().find(|c| c.id == id).map(stamp),
            Subject::ChangeRequest(_) => {
                self.change_requests.iter().find(|c| c.id == id).map(stamp)
            }
        }
    }

    /// The fingerprint of all the entries of one kind: the part of the model a rule about `kind` judges.
    pub fn scope_stamp(&self, kind: SubjectKind) -> Digest {
        match kind {
            SubjectKind::Context => {
                stamp(&self.contexts.iter().map(|c| c.head()).collect::<Vec<_>>())
            }
            SubjectKind::Interface => stamp(&self.interfaces().collect::<Vec<_>>()),
            SubjectKind::Item => stamp(&self.items().collect::<Vec<_>>()),
            SubjectKind::Clause => stamp(&self.clauses().collect::<Vec<_>>()),
            SubjectKind::Case => stamp(&self.cases().collect::<Vec<_>>()),
            SubjectKind::ChangeRequest => stamp(&self.change_requests),
            SubjectKind::Project => stamp(&(&self.contexts, &self.change_requests)),
        }
    }

    fn interfaces(&self) -> impl Iterator<Item = &InterfaceView> {
        self.contexts.iter().flat_map(|c| c.interfaces.iter())
    }

    fn items(&self) -> impl Iterator<Item = &ItemView> {
        self.interfaces().flat_map(|i| i.items.iter())
    }

    fn clauses(&self) -> impl Iterator<Item = &ClauseView> {
        self.contexts
            .iter()
            .flat_map(|c| c.contracts.iter())
            .flat_map(|k| k.clauses.iter())
    }

    fn cases(&self) -> impl Iterator<Item = &CaseView> {
        self.contexts
            .iter()
            .flat_map(|c| c.harnesses.iter())
            .flat_map(|h| h.cases.iter())
    }
}
