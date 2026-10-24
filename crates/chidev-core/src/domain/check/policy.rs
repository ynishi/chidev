//! The policy: the level and the parameters each check runs with, resolved from layers. The builtin defaults come
//! first (what each descriptor declares), then the project's layer, then the person's; a later layer overrides an
//! earlier one. A layer is typed here and parsed from a file elsewhere, so that the domain reads no file.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use thiserror::Error;

use super::descriptor::{Descriptor, ParamDefault, ParamKind};
use super::level::Level;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, RuleIdError};

/// A parameter's value: a scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamValue {
    /// true or false.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A text.
    Str(String),
}

impl ParamValue {
    /// The kind of the value.
    pub fn kind(&self) -> ParamKind {
        match self {
            ParamValue::Bool(_) => ParamKind::Bool,
            ParamValue::Int(_) => ParamKind::Int,
            ParamValue::Str(_) => ParamKind::Str,
        }
    }
}

impl From<ParamDefault> for ParamValue {
    fn from(d: ParamDefault) -> ParamValue {
        match d {
            ParamDefault::Bool(b) => ParamValue::Bool(b),
            ParamDefault::Int(i) => ParamValue::Int(i),
            ParamDefault::Str(s) => ParamValue::Str(s.to_string()),
        }
    }
}

/// The parameters a check runs with: every parameter its descriptor declares, each with a value of its kind.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Params(BTreeMap<String, ParamValue>);

impl Params {
    /// The value of `name`.
    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        self.0.get(name)
    }

    /// The boolean `name`; none when there is no such parameter or it is of another kind.
    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.0.get(name) {
            Some(ParamValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    /// The whole number `name`; none when there is no such parameter or it is of another kind.
    pub fn int(&self, name: &str) -> Option<i64> {
        match self.0.get(name) {
            Some(ParamValue::Int(i)) => Some(*i),
            _ => None,
        }
    }

    /// The text `name`; none when there is no such parameter or it is of another kind.
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(ParamValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// Every parameter, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ParamValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    fn defaults(d: &Descriptor) -> Params {
        Params(
            d.params
                .iter()
                .map(|p| (p.name.to_string(), ParamValue::from(p.default.clone())))
                .collect(),
        )
    }
}

/// Whom a layer comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The project's file, kept with the project.
    Project,
    /// The person's file, kept to themselves.
    Person,
}

impl Origin {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Origin::Project => "project",
            Origin::Person => "person",
        }
    }
}

/// What a layer sets for a check or a family: a level, parameters, or both.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Setting {
    /// The level; none leaves it as it is.
    pub level: Option<Level>,
    /// Parameters to set, by name.
    pub params: BTreeMap<String, ParamValue>,
}

/// What a setting is for: one check, or every check whose id starts with a prefix that ends in `.` (`cite.`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Selector {
    /// One check.
    Check(RuleId),
    /// A family: the prefix, with its `.`.
    Family(String),
}

impl Selector {
    /// A key of a policy file: a rule id, or a prefix that ends in `.` and is made of what a rule id is made of.
    pub fn parse(key: &str) -> Result<Selector, RuleIdError> {
        match key.strip_suffix('.') {
            Some(stem) => {
                RuleId::try_from(stem)?;
                Ok(Selector::Family(key.to_string()))
            }
            None => RuleId::try_from(key).map(Selector::Check),
        }
    }

    fn describe(&self) -> String {
        match self {
            Selector::Check(id) => id.to_string(),
            Selector::Family(p) => p.clone(),
        }
    }
}

/// One layer of a policy: the settings a file gave, in the order it gave them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// Whom it comes from.
    pub origin: Origin,
    /// The settings.
    pub entries: Vec<(Selector, Setting)>,
}

/// Why the layers make no policy. Nothing is run under a policy that cannot be read: a rule the person thought
/// was on must not be quietly off.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// The layer names a check, or a family, that does not exist.
    #[error("the {origin} policy names `{name}`, which is no check", origin = .origin.word())]
    UnknownCheck {
        /// The layer.
        origin: Origin,
        /// The key.
        name: String,
    },
    /// The layer sets a parameter the check does not declare.
    #[error("the {origin} policy sets `{param}` on `{name}`, which declares no such parameter", origin = .origin.word())]
    UndeclaredParam {
        /// The layer.
        origin: Origin,
        /// The key.
        name: String,
        /// The parameter.
        param: String,
    },
    /// The layer sets a parameter to a value of another kind.
    #[error("the {origin} policy sets `{param}` on `{name}` to a {got}; it is a {expected}", origin = .origin.word(), got = .got.word(), expected = .expected.word())]
    ParamKind {
        /// The layer.
        origin: Origin,
        /// The key.
        name: String,
        /// The parameter.
        param: String,
        /// The kind it has.
        expected: ParamKind,
        /// The kind it was given.
        got: ParamKind,
    },
    /// The layer sets a mandatory check below `error`.
    #[error("the {origin} policy sets the mandatory check `{check}` to {level}; it stays at error", origin = .origin.word())]
    Mandatory {
        /// The layer.
        origin: Origin,
        /// The check.
        check: RuleId,
        /// The level asked.
        level: Level,
    },
}

/// What one check runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The level.
    pub level: Level,
    /// The parameters.
    pub params: Params,
}

/// The level and parameters of every check, after the layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    checks: BTreeMap<RuleId, Resolved>,
}

impl Policy {
    /// What the descriptors declare, with no layer on top.
    pub fn defaults<'a>(descriptors: impl IntoIterator<Item = &'a Descriptor>) -> Policy {
        Policy {
            checks: descriptors
                .into_iter()
                .map(|d| {
                    (
                        d.id.clone(),
                        Resolved {
                            level: d.default_level,
                            params: Params::defaults(d),
                        },
                    )
                })
                .collect(),
        }
    }

    /// The defaults of `descriptors` with `layers` applied in order. Within a layer a family is applied before the
    /// check it contains (the longer prefix later), so the check wins. A family leaves a mandatory check at
    /// `error`; a layer that names a mandatory check itself and sets it lower is refused, as it is for a layer
    /// that names what does not exist or does not fit.
    pub fn resolve(descriptors: &[&Descriptor], layers: &[Layer]) -> Result<Policy, PolicyError> {
        let mut policy = Policy::defaults(descriptors.iter().copied());
        for layer in layers {
            let mut entries: Vec<&(Selector, Setting)> = layer.entries.iter().collect();
            // Families by length of prefix, then the checks: the more particular, the later.
            entries.sort_by_key(|(s, _)| match s {
                Selector::Family(p) => (0, p.len()),
                Selector::Check(_) => (1, 0),
            });
            // Everything is checked before anything is set, so an error leaves nothing half applied.
            let mut planned = Vec::new();
            for (selector, setting) in entries {
                let matched: Vec<&Descriptor> = descriptors
                    .iter()
                    .copied()
                    .filter(|d| match selector {
                        Selector::Check(id) => d.id == *id,
                        Selector::Family(p) => d.id.in_family(p),
                    })
                    .collect();
                if matched.is_empty() {
                    return Err(PolicyError::UnknownCheck {
                        origin: layer.origin,
                        name: selector.describe(),
                    });
                }
                for (param, value) in &setting.params {
                    let mut declared = false;
                    for d in &matched {
                        let Some(spec) = d.param(param) else { continue };
                        declared = true;
                        if spec.default.kind() != value.kind() {
                            return Err(PolicyError::ParamKind {
                                origin: layer.origin,
                                name: selector.describe(),
                                param: param.clone(),
                                expected: spec.default.kind(),
                                got: value.kind(),
                            });
                        }
                    }
                    if !declared {
                        return Err(PolicyError::UndeclaredParam {
                            origin: layer.origin,
                            name: selector.describe(),
                            param: param.clone(),
                        });
                    }
                }
                if let (Selector::Check(_), Some(level)) = (selector, setting.level) {
                    for d in matched.iter().filter(|d| d.mandatory) {
                        if level < Level::Error {
                            return Err(PolicyError::Mandatory {
                                origin: layer.origin,
                                check: d.id.clone(),
                                level,
                            });
                        }
                    }
                }
                planned.push((selector, setting, matched));
            }
            for (selector, setting, matched) in planned {
                for d in matched {
                    let Some(now) = policy.checks.get_mut(&d.id) else {
                        continue;
                    };
                    if let Some(level) = setting.level
                        && !(d.mandatory && matches!(selector, Selector::Family(_)))
                    {
                        now.level = level;
                    }
                    for (param, value) in &setting.params {
                        if d.param(param).is_some() {
                            now.params.0.insert(param.clone(), value.clone());
                        }
                    }
                }
            }
        }
        Ok(policy)
    }

    /// The level and parameters of the check `id`.
    pub fn of(&self, id: &RuleId) -> Option<&Resolved> {
        self.checks.get(id)
    }

    /// The level of the check `id`; off when the policy does not know it.
    pub fn level(&self, id: &RuleId) -> Level {
        self.of(id).map_or(Level::Off, |r| r.level)
    }

    /// A fingerprint of the policy: the same levels and parameters give the same digest, in any order the layers
    /// were written, so a record can say which policy a run was made under.
    pub fn hash(&self) -> Digest {
        let mut text = String::new();
        for (id, r) in &self.checks {
            // Writing to a String does not fail.
            let _ = writeln!(text, "{id} {}", r.level);
            for (name, value) in r.params.iter() {
                let _ = match value {
                    ParamValue::Bool(b) => writeln!(text, "  {name}=bool:{b}"),
                    ParamValue::Int(i) => writeln!(text, "  {name}=int:{i}"),
                    ParamValue::Str(s) => writeln!(text, "  {name}=str:{s:?}"),
                };
            }
        }
        Digest::of(text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::check::descriptor::{Executor, ParamSpec};
    use crate::domain::check::level::Hook;

    static PARAMS: [ParamSpec; 2] = [
        ParamSpec {
            name: std::borrow::Cow::Borrowed("max"),
            summary: std::borrow::Cow::Borrowed("how many"),
            default: ParamDefault::Int(3),
        },
        ParamSpec {
            name: std::borrow::Cow::Borrowed("strict"),
            summary: std::borrow::Cow::Borrowed("whether"),
            default: ParamDefault::Bool(false),
        },
    ];

    fn descriptor(
        id: RuleId,
        level: Level,
        mandatory: bool,
        params: &'static [ParamSpec],
    ) -> Descriptor {
        Descriptor {
            id,
            summary: "".into(),
            help: "".into(),
            hooks: [Hook::Check][..].into(),
            default_level: level,
            mandatory,
            params: params.into(),
            executor: Executor::Builtin,
        }
    }

    /// `a.loose` (warning, with parameters), `a.must` (mandatory, error) and `b.other` (note).
    fn descriptors() -> [Descriptor; 3] {
        [
            descriptor(
                RuleId::from_static("a.loose"),
                Level::Warning,
                false,
                &PARAMS,
            ),
            descriptor(RuleId::from_static("a.must"), Level::Error, true, &[]),
            descriptor(RuleId::from_static("b.other"), Level::Note, false, &[]),
        ]
    }

    fn level(l: Level) -> Setting {
        Setting {
            level: Some(l),
            ..Setting::default()
        }
    }

    fn layer(origin: Origin, entries: &[(&str, Setting)]) -> Layer {
        Layer {
            origin,
            entries: entries
                .iter()
                .map(|(k, s)| (Selector::parse(k).unwrap(), s.clone()))
                .collect(),
        }
    }

    fn resolve(layers: &[Layer]) -> Result<Policy, PolicyError> {
        let d = descriptors();
        let refs: Vec<&Descriptor> = d.iter().collect();
        Policy::resolve(&refs, layers)
    }

    fn id(s: &str) -> RuleId {
        RuleId::try_from(s).unwrap()
    }

    #[test]
    fn without_a_layer_each_check_has_what_it_declares() {
        let p = resolve(&[]).unwrap();
        assert_eq!(p.level(&id("a.loose")), Level::Warning);
        assert_eq!(p.level(&id("a.must")), Level::Error);
        assert_eq!(p.level(&id("b.other")), Level::Note);
        assert_eq!(p.of(&id("a.loose")).unwrap().params.int("max"), Some(3));
        assert_eq!(p.level(&id("nobody")), Level::Off);
    }

    #[test]
    fn a_later_layer_overrides_an_earlier_one() {
        let p = resolve(&[
            layer(Origin::Project, &[("a.loose", level(Level::Error))]),
            layer(Origin::Person, &[("a.loose", level(Level::Off))]),
        ])
        .unwrap();
        assert_eq!(p.level(&id("a.loose")), Level::Off);
    }

    #[test]
    fn a_family_sets_its_checks_and_the_check_named_wins_over_it() {
        let p = resolve(&[layer(
            Origin::Project,
            &[("a.loose", level(Level::Note)), ("a.", level(Level::Off))],
        )])
        .unwrap();
        assert_eq!(p.level(&id("a.loose")), Level::Note);
        assert_eq!(p.level(&id("b.other")), Level::Note);
    }

    #[test]
    fn a_family_leaves_a_mandatory_check_at_error() {
        let p = resolve(&[layer(Origin::Person, &[("a.", level(Level::Off))])]).unwrap();
        assert_eq!(p.level(&id("a.must")), Level::Error);
        assert_eq!(p.level(&id("a.loose")), Level::Off);
    }

    #[test]
    fn a_mandatory_check_named_below_error_is_refused_whatever_the_layer() {
        for l in [Level::Off, Level::Note, Level::Warning] {
            assert_eq!(
                resolve(&[layer(Origin::Project, &[("a.must", level(l))])]),
                Err(PolicyError::Mandatory {
                    origin: Origin::Project,
                    check: id("a.must"),
                    level: l
                })
            );
        }
        // Naming it at error is allowed: it says what is already so.
        assert!(resolve(&[layer(Origin::Person, &[("a.must", level(Level::Error))])]).is_ok());
    }

    #[test]
    fn a_layer_naming_what_does_not_exist_is_refused() {
        assert_eq!(
            resolve(&[layer(Origin::Project, &[("a.nothing", level(Level::Off))])]),
            Err(PolicyError::UnknownCheck {
                origin: Origin::Project,
                name: "a.nothing".to_string()
            })
        );
        assert!(matches!(
            resolve(&[layer(Origin::Person, &[("z.", level(Level::Off))])]),
            Err(PolicyError::UnknownCheck { .. })
        ));
    }

    #[test]
    fn parameters_are_set_when_declared_and_of_their_kind() {
        let set = |name: &str, v: ParamValue| Setting {
            level: None,
            params: BTreeMap::from([(name.to_string(), v)]),
        };
        let p = resolve(&[layer(
            Origin::Project,
            &[("a.loose", set("max", ParamValue::Int(9)))],
        )])
        .unwrap();
        assert_eq!(p.of(&id("a.loose")).unwrap().params.int("max"), Some(9));
        assert_eq!(
            p.of(&id("a.loose")).unwrap().params.bool("strict"),
            Some(false)
        );
        assert!(matches!(
            resolve(&[layer(
                Origin::Project,
                &[("a.loose", set("min", ParamValue::Int(1)))]
            )]),
            Err(PolicyError::UndeclaredParam { .. })
        ));
        assert!(matches!(
            resolve(&[layer(
                Origin::Project,
                &[("a.loose", set("max", ParamValue::Str("9".to_string())))]
            )]),
            Err(PolicyError::ParamKind {
                expected: ParamKind::Int,
                got: ParamKind::Str,
                ..
            })
        ));
        // A family sets a parameter on the checks that declare it, and is refused when none does.
        assert!(
            resolve(&[layer(
                Origin::Project,
                &[("a.", set("max", ParamValue::Int(2)))]
            )])
            .is_ok()
        );
        assert!(matches!(
            resolve(&[layer(
                Origin::Project,
                &[("b.", set("max", ParamValue::Int(2)))]
            )]),
            Err(PolicyError::UndeclaredParam { .. })
        ));
    }

    #[test]
    fn the_hash_follows_levels_and_parameters_and_nothing_else() {
        let same_a = resolve(&[layer(
            Origin::Project,
            &[
                ("a.loose", level(Level::Note)),
                ("b.other", level(Level::Error)),
            ],
        )])
        .unwrap();
        let same_b = resolve(&[
            layer(Origin::Project, &[("b.other", level(Level::Error))]),
            layer(Origin::Person, &[("a.loose", level(Level::Note))]),
        ])
        .unwrap();
        assert_eq!(same_a.hash(), same_b.hash());
        assert_ne!(same_a.hash(), resolve(&[]).unwrap().hash());
    }

    #[test]
    fn a_key_is_a_rule_id_or_a_family() {
        assert_eq!(
            Selector::parse("cite.").unwrap(),
            Selector::Family("cite.".to_string())
        );
        assert_eq!(
            Selector::parse("cite.stale").unwrap(),
            Selector::Check(id("cite.stale"))
        );
        assert!(Selector::parse(".").is_err());
        assert!(Selector::parse("Cite.").is_err());
    }
}
