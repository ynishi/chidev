//! Port for external checks. Transport is an adapter's concern; validation and policy belong to the app.

use std::collections::BTreeMap;
use std::future::Future;

use crate::app::model_view::ModelSlice;
use crate::domain::check::{Level, Moment};
use crate::domain::vo::finding::{RuleId, SubjectKind};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

/// A restricted, versioned view: omitted sections were not requested, not empty. It is the [`ModelSlice`] of the
/// sections a rule reads, written as an external program has always received it.
///
/// A program reads it as JSON in another process, so there is no way to read it back into this type.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalModelView(ModelSlice);

impl ExternalModelView {
    /// The slice a program is sent.
    pub(in crate::app) fn new(slice: ModelSlice) -> Self {
        Self(slice)
    }

    /// The slice a program is sent.
    pub(in crate::app) fn slice(&self) -> &ModelSlice {
        &self.0
    }

    /// A model of no Context and no change request, for a test that needs a call to carry.
    #[cfg(test)]
    pub(crate) fn empty(version: u32) -> Self {
        Self(ModelSlice::empty(version))
    }
}

/// Writes the slice with the bytes it had before it was typed: the fields of the top level in the order they are
/// declared (`version`, `contexts`, `change_requests`, `cruxes`, the last two only when they were requested), and each
/// Context, the change requests and the Cruxes through a JSON value, whose keys are in the order of their names.
///
/// The bytes are not for show. The digest of this document is kept with an external run as its `input_stamp` and
/// compared with the digest of the document of the model now, so a different order of keys would make every saved
/// run stale at once.
impl Serialize for ExternalModelView {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let slice = &self.0;
        let contexts = slice
            .contexts()
            .iter()
            .map(by_name::<_, S::Error>)
            .collect::<Result<Vec<_>, _>>()?;
        let len = 2
            + usize::from(slice.change_requests().is_some())
            + usize::from(slice.cruxes().is_some());
        let mut model = serializer.serialize_struct("ExternalModelView", len)?;
        model.serialize_field("version", &slice.version())?;
        model.serialize_field("contexts", &contexts)?;
        if let Some(requests) = slice.change_requests() {
            model.serialize_field("change_requests", &by_name::<_, S::Error>(requests)?)?;
        }
        if let Some(cruxes) = slice.cruxes() {
            model.serialize_field("cruxes", &by_name::<_, S::Error>(cruxes)?)?;
        }
        model.end()
    }
}

/// The schema of the slice: the order of the keys is the only thing [`Serialize`] above changes, and a schema does not
/// state an order.
impl JsonSchema for ExternalModelView {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ExternalModelView".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ModelSlice::json_schema(generator)
    }
}

/// `value` as a JSON value, which keeps the keys of an object in the order of their names.
fn by_name<T: Serialize + ?Sized, E: serde::ser::Error>(value: &T) -> Result<serde_json::Value, E> {
    serde_json::to_value(value).map_err(E::custom)
}

/// The version of the request a program is sent. Version 2 calls the moment of the call `moment` (it was `hook`) and
/// says `edit` where version 1 said `put`; nothing else in the request changed. Which use case asked is not in it:
/// a program is told when it is asked (the moment), not by whom, so that the names of the use cases are not a
/// protocol a program can depend on.
pub const REQUEST_PROTOCOL: u32 = 2;

/// The versions of a reply that are accepted. The reply did not change between version 1 and 2, so a program that
/// answers as it did keeps working once it reads `moment`; any other version is a failed run.
pub const REPLY_PROTOCOLS: [u32; 2] = [1, 2];

/// The kinds of subject a finding may name: every kind but the Project, which has no id; a finding about it names no
/// subject.
pub fn subject_kinds() -> impl Iterator<Item = SubjectKind> {
    SubjectKind::ALL
        .into_iter()
        .filter(|k| *k != SubjectKind::Project)
}

/// The levels a finding may give: every level a check reports at, that is every one but `off`.
pub fn finding_levels() -> impl Iterator<Item = Level> {
    Level::ALL.into_iter().filter(|l| *l != Level::Off)
}

/// The schema of a request's `protocol`: [`REQUEST_PROTOCOL`].
fn request_protocol_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "integer", "const": REQUEST_PROTOCOL })
}

/// The schema of a reply's `protocol`: one of [`REPLY_PROTOCOLS`].
fn reply_protocol_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "integer", "enum": REPLY_PROTOCOLS })
}

/// The schema of `moment`: the words of the moments.
fn moment_schema(_: &mut SchemaGenerator) -> Schema {
    let words: Vec<&str> = Moment::ALL.iter().map(|m| m.word()).collect();
    json_schema!({ "type": "string", "enum": words })
}

/// The schema of a subject's `kind`: the word of one of [`subject_kinds`].
fn subject_kind_schema(_: &mut SchemaGenerator) -> Schema {
    let words: Vec<&str> = subject_kinds().map(SubjectKind::word).collect();
    json_schema!({ "type": "string", "enum": words })
}

/// The schema of a finding's `level`: the word of one of [`finding_levels`], or none.
fn finding_level_schema(_: &mut SchemaGenerator) -> Schema {
    let words: Vec<serde_json::Value> = finding_levels()
        .map(|l| serde_json::Value::from(l.word()))
        .chain([serde_json::Value::Null])
        .collect();
    json_schema!({ "enum": words })
}

/// One process request, also its stdin JSON format.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExternalCall {
    /// The version of the request.
    #[schemars(schema_with = "request_protocol_schema")]
    pub protocol: u32,
    /// The moment the program is asked at: `edit`, `pin`, `resolve` or `check`.
    #[schemars(schema_with = "moment_schema")]
    pub moment: String,
    /// Exactly one rule in the first protocol implementation.
    pub rules: Vec<String>,
    /// Store revision before the proposed change.
    pub read_at: u64,
    /// Resolved policy parameters for this rule.
    pub params: BTreeMap<String, serde_json::Value>,
    /// The proposed model, restricted by the rule's reads.
    pub model: ExternalModelView,
}

/// A subject by kind and UUID, validated against the sent model by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalSubject {
    /// The kind of the subject: a kind of the model the program was sent whose ids it can be checked against.
    #[schemars(schema_with = "subject_kind_schema")]
    pub kind: String,
    /// Subject UUID.
    pub id: String,
}

/// An untrusted finding from a program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalFinding {
    /// The requested rule.
    pub rule: String,
    /// None means the whole project.
    #[serde(default)]
    pub subject: Option<ExternalSubject>,
    /// Nonblank explanation.
    pub message: String,
    /// Optional severity, validated and bounded by policy.
    #[serde(default)]
    #[schemars(schema_with = "finding_level_schema")]
    pub level: Option<String>,
}

/// The stdout JSON format.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalReply {
    /// The version of the reply: one of the versions accepted.
    #[schemars(schema_with = "reply_protocol_schema")]
    pub protocol: u32,
    /// Every finding; empty means a successful check with no findings.
    pub findings: Vec<ExternalFinding>,
}

/// Evidence available even when execution failed before a process started.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionEvidence {
    /// Resolved executable path, if resolution succeeded. The file behind it is not digested: nothing would compare
    /// a digest of it, and reading a large program on every run counted against the run's time limit.
    pub command: Option<String>,
    /// Digest of process configuration.
    pub config_digest: String,
    /// Exit code when available (a signal has none).
    pub exit_code: Option<i32>,
    /// Bounded stderr diagnostics.
    pub stderr: String,
}

/// A failed attempt, never confused with an empty successful response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct ExternalFailure {
    /// Stable failure category: spawn, timeout, exit, limit, protocol, or transport.
    pub kind: String,
    /// Diagnostic detail.
    pub message: String,
}

/// One completed attempt and the evidence of what ran.
#[derive(Debug)]
pub struct ExternalAttempt {
    /// Execution identity and diagnostics.
    pub evidence: ExecutionEvidence,
    /// Decoded response or explicit failure.
    pub result: Result<ExternalReply, ExternalFailure>,
}

/// Consumer-owned port. Implementations must bound output and stop and reap their process group on cancellation.
pub trait ExternalExecutor: std::fmt::Debug + Send + Sync {
    /// Runs a configured rule once. It must not retry a process or modify the store.
    fn execute(
        &self,
        rule: &RuleId,
        call: ExternalCall,
    ) -> impl Future<Output = ExternalAttempt> + Send;
}

/// The default executor for applications that have no registered external rules.
#[derive(Debug)]
pub struct NoExternal;

impl ExternalExecutor for NoExternal {
    async fn execute(&self, _: &RuleId, _: ExternalCall) -> ExternalAttempt {
        ExternalAttempt {
            evidence: ExecutionEvidence::default(),
            result: Err(ExternalFailure {
                kind: "unconfigured".into(),
                message: "no external executor is configured".into(),
            }),
        }
    }
}

/// A normalized finding kept with the last execution of a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalStoredFinding {
    /// Subject validated against the input view.
    pub subject: Option<ExternalSubject>,
    /// Explanation.
    pub message: String,
    /// Effective severity.
    pub level: String,
}

/// Latest execution of one rule, atomically kept with the change it checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalReport {
    /// Rule id.
    pub rule: String,
    /// Digest of the restricted candidate model.
    pub input_stamp: String,
    /// Digest of the rule, effective policy and executor configuration.
    pub config_stamp: String,
    /// Normalized findings, including a project finding on execution failure.
    pub findings: Vec<ExternalStoredFinding>,
    /// Execution evidence.
    pub evidence: ExecutionEvidence,
    /// Failure detail when the checker could not run or answer correctly.
    pub failure: Option<String>,
}
