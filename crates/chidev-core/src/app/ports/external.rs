//! Port for external checks. Transport is an adapter's concern; validation and policy belong to the app.

use std::collections::BTreeMap;
use std::future::Future;

use crate::domain::vo::finding::RuleId;
use serde::{Deserialize, Serialize};

/// A restricted, versioned view: omitted sections were not requested, not empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalModelView {
    /// Model view format, independent of the process protocol.
    pub version: u32,
    /// Context metadata and the requested asset sections only.
    pub contexts: Vec<serde_json::Value>,
    /// Change requests, only when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_requests: Option<serde_json::Value>,
}

/// One process request, also its stdin JSON format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalCall {
    /// Process protocol version.
    pub protocol: u32,
    /// Hook name.
    pub hook: String,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalSubject {
    /// Subject kind.
    pub kind: String,
    /// Subject UUID.
    pub id: String,
}

/// An untrusted finding from a program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub level: Option<String>,
}

/// The stdout JSON format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalReply {
    /// Must be one.
    pub protocol: u32,
    /// Every finding; empty means a successful check with no findings.
    pub findings: Vec<ExternalFinding>,
}

/// Evidence available even when execution failed before a process started.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionEvidence {
    /// Resolved executable path, if resolution succeeded.
    pub command: Option<String>,
    /// SHA-256 of that executable, not its interpreter's inputs or dependencies.
    pub command_digest: Option<String>,
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
