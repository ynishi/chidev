//! An external check's rules, independent of its process transport.

use super::{Descriptor, Executor, Hook, Level};
use crate::domain::vo::finding::RuleId;
use std::borrow::Cow;

/// A configured rule whose execution is supplied through an app port.
#[derive(Debug, Clone)]
pub struct ExternalRule {
    descriptor: Descriptor,
    reads: Vec<String>,
    config_digest: String,
}

impl ExternalRule {
    /// Validates a rule's namespace, hooks and the sections it reads. A gate cannot be lowered by policy.
    pub fn new(
        id: RuleId,
        hooks: Vec<Hook>,
        reads: Vec<String>,
        gate: bool,
        config_digest: String,
    ) -> Result<Self, String> {
        if !id.in_family("ext.") || id.as_str() == "ext." {
            return Err("an external id starts with ext. and has a name".into());
        }
        if hooks.is_empty() {
            return Err(format!("{id} has no hooks"));
        }
        if reads.is_empty()
            || reads.iter().any(|r| {
                !["contracts", "interfaces", "harnesses", "change_requests"].contains(&r.as_str())
            })
        {
            return Err(format!("{id} has invalid reads"));
        }
        Ok(Self {
            descriptor: Descriptor {
                summary: Cow::Owned(id.to_string()),
                help: Cow::Borrowed("Run the project's external checker."),
                id,
                hooks: Cow::Owned(hooks),
                default_level: if gate { Level::Error } else { Level::Warning },
                mandatory: gate,
                params: std::borrow::Cow::Borrowed(&[]),
                executor: Executor::External,
            },
            reads,
            config_digest,
        })
    }
    /// Declares scalar parameter defaults; policy overrides must preserve their types.
    pub fn with_params(
        mut self,
        params: std::collections::BTreeMap<String, super::ParamValue>,
    ) -> Result<Self, String> {
        use super::{ParamDefault, ParamSpec, ParamValue};
        let mut specs = Vec::new();
        for (name, value) in params {
            if name.trim().is_empty() {
                return Err("a parameter name is nonblank".into());
            }
            let default = match value {
                ParamValue::Bool(v) => ParamDefault::Bool(v),
                ParamValue::Int(v) => ParamDefault::Int(v),
                ParamValue::Str(v) => ParamDefault::Str(Cow::Owned(v)),
            };
            specs.push(ParamSpec {
                name: Cow::Owned(name),
                summary: Cow::Borrowed("External checker parameter"),
                default,
            });
        }
        self.descriptor.params = Cow::Owned(specs);
        Ok(self)
    }

    /// The common check descriptor.
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    /// Model sections sent to the program.
    pub fn reads(&self) -> &[String] {
        &self.reads
    }
    /// Whether any finding or execution failure refuses a change.
    pub fn gate(&self) -> bool {
        self.descriptor.mandatory
    }
    /// Identity of the configured process.
    pub fn config_digest(&self) -> &str {
        &self.config_digest
    }
}
