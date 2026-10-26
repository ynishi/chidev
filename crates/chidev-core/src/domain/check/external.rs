//! An external check's rules, independent of its process transport.

use super::{Descriptor, Executor, Level, Moment};
use crate::domain::vo::finding::RuleId;
use crate::domain::vo::model_section::ModelSection;
use std::borrow::Cow;

/// A configured rule whose execution is supplied through an app port.
#[derive(Debug, Clone)]
pub struct ExternalRule {
    descriptor: Descriptor,
    reads: Vec<ModelSection>,
    config_digest: String,
}

impl ExternalRule {
    /// Validates a rule's namespace, moments and the sections it reads (at least one). A gate cannot be lowered by
    /// policy. `reads` is kept as written, in order and with repeats: a saved run stamps it as it was configured.
    pub fn new(
        id: RuleId,
        moments: Vec<Moment>,
        reads: Vec<ModelSection>,
        gate: bool,
        config_digest: String,
    ) -> Result<Self, String> {
        if !id.in_family("ext.") || id.as_str() == "ext." {
            return Err("an external id starts with ext. and has a name".into());
        }
        if moments.is_empty() {
            return Err(format!("{id} has no moments"));
        }
        if reads.is_empty() {
            return Err(format!("{id} has no reads"));
        }
        Ok(Self {
            descriptor: Descriptor {
                summary: Cow::Owned(id.to_string()),
                help: Cow::Borrowed("Run the project's external checker."),
                id,
                moments: Cow::Owned(moments),
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
    /// Model sections sent to the program, as configured.
    pub fn reads(&self) -> &[ModelSection] {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(reads: Vec<ModelSection>) -> Result<ExternalRule, String> {
        ExternalRule::new(
            RuleId::from_static("ext.test"),
            vec![Moment::Check],
            reads,
            false,
            "config".into(),
        )
    }

    #[test]
    fn a_rule_reads_at_least_one_section() {
        let error = rule(vec![]).unwrap_err();
        assert!(error.contains("reads"), "{error}");
    }

    #[test]
    fn the_reads_are_kept_in_the_order_and_with_the_repeats_they_were_written() {
        use ModelSection::*;
        let reads = vec![Interfaces, Contracts, Interfaces];
        assert_eq!(rule(reads.clone()).unwrap().reads(), reads);
    }
}
