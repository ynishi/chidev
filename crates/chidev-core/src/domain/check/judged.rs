//! A judged rule: a check written as data, in the form of the Arch Review Rules (a question, when it is a finding,
//! when it is not, how to fix it, and the words to look for), which an agent judges one rule at a time. chidev does
//! not judge: it keeps the rule, hands the agent the rule and a view of the model, and records what the agent
//! reports. So a judged rule has a [`Descriptor`] like any check (the registry, the policy and the listing hold it
//! the same way) and the text the agent reads; it has no `run`.

use std::borrow::Cow;

use thiserror::Error;

use super::descriptor::{Descriptor, Executor};
use super::level::{Hook, Level};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, Subject, SubjectKind};

/// The prefix of a judged rule's id; no builtin check has it, so the two never share an id.
pub const ID_PREFIX: &str = "judged.";

/// Why a draft is not a judged rule.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JudgedRuleError {
    /// The id does not start with `judged.`.
    #[error("the id `{0}` of a judged rule starts with `judged.`")]
    NotJudgedId(RuleId),
    /// A text the agent reads is empty.
    #[error("the judged rule `{rule}` has an empty `{field}`")]
    Blank {
        /// The rule.
        rule: RuleId,
        /// The field.
        field: &'static str,
    },
    /// It belongs to no hook, so no one would be asked to judge it.
    #[error("the judged rule `{0}` belongs to no hook")]
    NoHooks(RuleId),
    /// It asks to be mandatory. A mandatory check refuses the use case that finds something, and a judged rule's
    /// result arrives after the use case, so it can refuse nothing.
    #[error(
        "the judged rule `{0}` cannot be mandatory: its result arrives after the use case, so it cannot block one"
    )]
    Mandatory(RuleId),
}

/// What a judged rule is written as, before its rules are checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedDraft {
    /// The rule's id.
    pub id: RuleId,
    /// What it finds, in a line.
    pub summary: String,
    /// The question the agent answers about each subject.
    pub question: String,
    /// When the answer is a finding.
    pub true_when: String,
    /// When it is not, and the exceptions.
    pub false_when: String,
    /// What to do about a finding.
    pub fix: String,
    /// Literal words that hint where to look; they narrow the agent's reading, they decide nothing.
    pub when: Vec<String>,
    /// The hooks it belongs to: at which of these moments the agent is asked to judge it.
    pub hooks: Vec<Hook>,
    /// The level it has when no policy says otherwise.
    pub level: Level,
    /// What it judges: a finding about this kind of thing.
    pub subject: SubjectKind,
    /// Whether the file asked for the rule to be mandatory; refused.
    pub mandatory: bool,
}

/// A rule an agent judges. The only way to make one is [`JudgedRule::new`], which refuses what cannot work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedRule {
    descriptor: Descriptor,
    question: String,
    true_when: String,
    false_when: String,
    fix: String,
    when: Vec<String>,
    subject: SubjectKind,
}

impl JudgedRule {
    /// The rule `draft` says. Its descriptor is owned (it comes from a file), is never mandatory, and takes no
    /// parameters.
    pub fn new(draft: JudgedDraft) -> Result<JudgedRule, JudgedRuleError> {
        let JudgedDraft {
            id,
            summary,
            question,
            true_when,
            false_when,
            fix,
            when,
            mut hooks,
            level,
            subject,
            mandatory,
        } = draft;
        if !id.in_family(ID_PREFIX) {
            return Err(JudgedRuleError::NotJudgedId(id));
        }
        if mandatory {
            return Err(JudgedRuleError::Mandatory(id));
        }
        for (field, text) in [
            ("summary", &summary),
            ("question", &question),
            ("true", &true_when),
            ("false", &false_when),
            ("fix", &fix),
        ] {
            if text.trim().is_empty() {
                return Err(JudgedRuleError::Blank { rule: id, field });
            }
        }
        if hooks.is_empty() {
            return Err(JudgedRuleError::NoHooks(id));
        }
        hooks.sort();
        hooks.dedup();
        Ok(JudgedRule {
            descriptor: Descriptor {
                id,
                summary: Cow::Owned(summary),
                help: Cow::Owned(fix.clone()),
                hooks: Cow::Owned(hooks),
                default_level: level,
                mandatory: false,
                params: std::borrow::Cow::Borrowed(&[]),
                executor: Executor::Judged,
            },
            question,
            true_when,
            false_when,
            fix,
            when,
            subject,
        })
    }

    /// Identifies what decides the judgment's answer, independently of its source file or effective policy.
    ///
    /// Version 2 hashes length-prefixed UTF-8 fields in this order: `chidev.judged-rule/v2`, id, subject,
    /// question, true, false. A field is its decimal byte length, `:`, then its bytes; text is exact.
    /// The summary, fix, `when` hints, hooks and default level are left out: they describe, guide, schedule or
    /// weight the rule but do not change what is a finding, so editing them must not make saved judgments stale
    /// (policy level overrides are excluded for the same reason). Comments and TOML formatting do not matter.
    /// Changing this encoding requires a new format marker so existing records cannot silently acquire another
    /// meaning.
    pub fn digest(&self) -> Digest {
        fn append(bytes: &mut Vec<u8>, value: &str) {
            bytes.extend_from_slice(value.len().to_string().as_bytes());
            bytes.push(b':');
            bytes.extend_from_slice(value.as_bytes());
        }
        let mut bytes = Vec::new();
        for value in [
            "chidev.judged-rule/v2",
            self.id().as_str(),
            self.subject().word(),
            self.question(),
            self.true_when(),
            self.false_when(),
        ] {
            append(&mut bytes, value);
        }
        Digest::of(&bytes)
    }

    /// What it says about itself, as every check does.
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    /// The rule's id.
    pub fn id(&self) -> &RuleId {
        &self.descriptor.id
    }

    /// The question the agent answers about each subject.
    pub fn question(&self) -> &str {
        &self.question
    }

    /// When the answer is a finding.
    pub fn true_when(&self) -> &str {
        &self.true_when
    }

    /// When it is not, and the exceptions.
    pub fn false_when(&self) -> &str {
        &self.false_when
    }

    /// What to do about a finding.
    pub fn fix(&self) -> &str {
        &self.fix
    }

    /// The literal hint words.
    pub fn when(&self) -> &[String] {
        &self.when
    }

    /// What kind of thing it judges.
    pub fn subject(&self) -> SubjectKind {
        self.subject
    }

    /// Whether a finding about `subject` (none: about the Project) is the kind this rule judges.
    pub fn admits(&self, subject: Option<Subject>) -> bool {
        SubjectKind::of(subject) == self.subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{ClauseId, test_uuid};

    pub(crate) fn draft() -> JudgedDraft {
        JudgedDraft {
            id: RuleId::try_from("judged.vague-clause").unwrap(),
            summary: "a clause too vague to check".to_string(),
            question: "can a case be written from this clause alone?".to_string(),
            true_when: "the clause has no condition or value a case could check".to_string(),
            false_when: "the clause is a definition".to_string(),
            fix: "reword it with a condition and an observable result".to_string(),
            when: vec!["should".to_string()],
            hooks: vec![Hook::Put, Hook::Check, Hook::Put],
            level: Level::Warning,
            subject: SubjectKind::Clause,
            mandatory: false,
        }
    }

    #[test]
    fn a_draft_becomes_a_rule_with_an_owned_descriptor_that_is_judged_and_not_mandatory() {
        let r = JudgedRule::new(draft()).unwrap();
        let d = r.descriptor();
        assert_eq!(d.id.as_str(), "judged.vague-clause");
        assert_eq!(d.executor, Executor::Judged);
        assert!(!d.mandatory && d.params.is_empty());
        assert_eq!(d.default_level, Level::Warning);
        // listed once each, in the order of the hooks
        assert_eq!(&*d.hooks, [Hook::Put, Hook::Check]);
        assert_eq!(d.help, r.fix());
        assert_eq!(r.when(), ["should"]);
        assert_eq!(r.subject(), SubjectKind::Clause);
    }

    #[test]
    fn a_draft_that_cannot_work_is_refused_for_the_reason() {
        let with = |f: fn(&mut JudgedDraft)| {
            let mut d = draft();
            f(&mut d);
            JudgedRule::new(d).unwrap_err()
        };
        assert!(matches!(
            with(|d| d.id = RuleId::try_from("cite.vague").unwrap()),
            JudgedRuleError::NotJudgedId(_)
        ));
        assert!(matches!(
            with(|d| d.mandatory = true),
            JudgedRuleError::Mandatory(_)
        ));
        assert!(matches!(
            with(|d| d.hooks.clear()),
            JudgedRuleError::NoHooks(_)
        ));
        assert!(matches!(
            with(|d| d.question = "  ".to_string()),
            JudgedRuleError::Blank {
                field: "question",
                ..
            }
        ));
        assert!(matches!(
            with(|d| d.false_when.clear()),
            JudgedRuleError::Blank { field: "false", .. }
        ));
    }

    #[test]
    fn a_rule_admits_the_subjects_of_the_kind_it_judges() {
        let r = JudgedRule::new(draft()).unwrap();
        let clause = Subject::Clause(ClauseId::from_uuid(test_uuid(1)));
        assert!(r.admits(Some(clause)));
        assert!(!r.admits(None));
        let mut d = draft();
        d.subject = SubjectKind::Project;
        let project = JudgedRule::new(d).unwrap();
        assert!(project.admits(None) && !project.admits(Some(clause)));
    }
    #[test]
    fn every_deciding_field_participates_and_the_others_do_not() {
        let baseline = JudgedRule::new(draft()).unwrap().digest();
        let deciding: [fn(&mut JudgedDraft); 5] = [
            |d| d.id = RuleId::try_from("judged.other").unwrap(),
            |d| d.subject = SubjectKind::Project,
            |d| d.question.push('!'),
            |d| d.true_when.push('!'),
            |d| d.false_when.push('!'),
        ];
        for change in deciding {
            let mut d = draft();
            change(&mut d);
            assert_ne!(JudgedRule::new(d).unwrap().digest(), baseline);
        }
        let others: [fn(&mut JudgedDraft); 6] = [
            |d| d.summary.push('!'),
            |d| d.fix.push('!'),
            |d| d.when[0].push('!'),
            |d| d.when.push("must".into()),
            |d| d.hooks.push(Hook::Resolve),
            |d| d.level = Level::Error,
        ];
        for change in others {
            let mut d = draft();
            change(&mut d);
            assert_eq!(JudgedRule::new(d).unwrap().digest(), baseline);
        }
    }

    #[test]
    fn field_boundaries_are_unambiguous_and_text_is_exact() {
        let mut a = draft();
        let mut b = a.clone();
        a.question = "ab".into();
        a.true_when = "c".into();
        b.question = "a".into();
        b.true_when = "bc".into();
        assert_ne!(
            JudgedRule::new(a).unwrap().digest(),
            JudgedRule::new(b).unwrap().digest()
        );
        let a = draft();
        let mut b = a.clone();
        b.question.push(' ');
        assert_ne!(
            JudgedRule::new(a).unwrap().digest(),
            JudgedRule::new(b).unwrap().digest()
        );
    }

    #[test]
    fn definition_digest_format_v2_is_stable() {
        assert_eq!(
            JudgedRule::new(draft()).unwrap().digest().to_string(),
            "351f833b6ac8496d6f6c5d81605a1550e774c6037a8f436f080d206c565533bd"
        );
    }
}
