//! The `[checks]` of the project's settings, read into the typed layers the domain resolves: the table of
//! `chidev.toml` (the project's choices, kept with it) and the table of the person's file (their own, not kept with
//! the project); a file without the table is an empty layer. The shape of the table:
//!
//! ```toml
//! [checks."cite."]            # a family: every check whose id starts with `cite.`
//! level = "note"
//!
//! [checks."pin.stale"]        # one check
//! level = "error"
//! [checks."pin.stale".params] # scalars only: a bool, an integer or a string
//! max = 3
//! ```
//!
//! The table is read here, at the edge; what a key may name and what a mandatory check may be set to are the
//! domain's rules ([`Policy::resolve`](crate::domain::check::Policy::resolve)), so it is checked against the checks
//! that exist when the registry is built.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::project::{Project, Section};
use crate::domain::check::policy::ParamValue;
use crate::domain::check::{Layer, Level, Origin, Selector, Setting};

/// Why the `[checks]` of a settings file is not a policy.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}: {message}", path.display())]
pub struct PolicyFileError {
    /// The settings file.
    pub path: PathBuf,
    /// What is wrong.
    pub message: String,
}

/// The layers of the policy for `project`: the project's, then the person's. A file without `[checks]` adds an empty
/// layer.
pub fn load(project: &Project) -> Result<Vec<Layer>, PolicyFileError> {
    let error = |path: &Path| {
        let path = path.to_path_buf();
        move |message| PolicyFileError { path, message }
    };
    Ok(vec![
        layer(project.section(Section::Checks), Origin::Project).map_err(error(project.file()))?,
        layer(project.person_section(Section::Checks), Origin::Person)
            .map_err(error(project.person_file()))?,
    ])
}

/// The layer a `[checks]` table is, for `origin`; no table is an empty layer.
pub(crate) fn layer(value: Option<&toml::Value>, origin: Origin) -> Result<Layer, String> {
    let mut entries = Vec::new();
    if let Some(value) = value {
        let toml::Value::Table(checks) = value else {
            return Err("`checks` is a table of checks".to_string());
        };
        for (name, setting) in checks {
            let selector = Selector::parse(name).map_err(|e| format!("checks.\"{name}\": {e}"))?;
            entries.push((selector, setting_of(name, setting)?));
        }
    }
    Ok(Layer { origin, entries })
}

/// The layer a text with only a `[checks]` table is, for `origin`; any other top-level key is refused.
pub fn parse(text: &str, origin: Origin) -> Result<Layer, String> {
    let doc: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    if let Some(key) = doc.keys().find(|k| k.as_str() != Section::Checks.name()) {
        return Err(format!(
            "`{key}` is not a section of a policy; there is `{}`",
            Section::Checks.name()
        ));
    }
    layer(doc.get(Section::Checks.name()), origin)
}

fn setting_of(name: &str, value: &toml::Value) -> Result<Setting, String> {
    let toml::Value::Table(table) = value else {
        return Err(format!(
            "checks.\"{name}\" is a table with `level` and `params`"
        ));
    };
    let mut setting = Setting::default();
    for (key, v) in table {
        match (key.as_str(), v) {
            ("level", toml::Value::String(word)) => {
                setting.level = Some(Level::from_word(word).ok_or_else(|| {
                    format!(
                        "checks.\"{name}\".level is `{word}`; it is off, note, warning or error"
                    )
                })?);
            }
            ("level", _) => return Err(format!("checks.\"{name}\".level is a string")),
            ("params", toml::Value::Table(params)) => {
                setting.params = params_of(name, params)?;
            }
            ("params", _) => return Err(format!("checks.\"{name}\".params is a table")),
            (other, _) => {
                return Err(format!(
                    "checks.\"{name}\".{other} is not a setting; there are `level` and `params`"
                ));
            }
        }
    }
    Ok(setting)
}

fn params_of(name: &str, params: &toml::Table) -> Result<BTreeMap<String, ParamValue>, String> {
    params
        .iter()
        .map(|(k, v)| {
            let value = match v {
                toml::Value::Boolean(b) => ParamValue::Bool(*b),
                toml::Value::Integer(i) => ParamValue::Int(*i),
                toml::Value::String(s) => ParamValue::Str(s.clone()),
                other => {
                    return Err(format!(
                        "checks.\"{name}\".params.{k} is a {}; a parameter is a bool, an integer or a string",
                        other.type_str()
                    ));
                }
            };
            Ok((k.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::finding::RuleId;

    #[test]
    fn a_file_is_read_into_a_layer() {
        let layer = parse(
            r#"
            [checks."cite."]
            level = "note"

            [checks."pin.stale"]
            level = "error"
            [checks."pin.stale".params]
            max = 3
            strict = true
            who = "me"
            "#,
            Origin::Project,
        )
        .unwrap();
        assert_eq!(layer.origin, Origin::Project);
        assert_eq!(layer.entries.len(), 2);
        assert_eq!(
            layer.entries[0],
            (
                Selector::Family("cite.".to_string()),
                Setting {
                    level: Some(Level::Note),
                    params: BTreeMap::new()
                }
            )
        );
        let (selector, setting) = &layer.entries[1];
        assert_eq!(
            *selector,
            Selector::Check(RuleId::try_from("pin.stale").unwrap())
        );
        assert_eq!(setting.level, Some(Level::Error));
        assert_eq!(setting.params["max"], ParamValue::Int(3));
        assert_eq!(setting.params["strict"], ParamValue::Bool(true));
        assert_eq!(setting.params["who"], ParamValue::Str("me".to_string()));
    }

    #[test]
    fn what_is_not_a_policy_is_refused_with_where() {
        for (text, expect) in [
            ("[other]\nx = 1", "not a section"),
            (
                "[checks]\n[external.\"ext.a\"]",
                "`external` is not a section",
            ),
            ("[measure]", "`measure` is not a section"),
            (
                "[checks.\"pin.stale\"]\nlevel = \"loud\"",
                "off, note, warning or error",
            ),
            ("[checks.\"pin.stale\"]\nlevel = 3", "level is a string"),
            ("[checks.\"pin.stale\"]\nseverity = \"x\"", "not a setting"),
            ("[checks.\"Pin\"]\nlevel = \"off\"", "Pin"),
            (
                "[checks.\"pin.stale\".params]\nx = [1]",
                "a parameter is a bool",
            ),
            ("[checks.\"pin.stale\".params]\nx = 1.5", "a float"),
            ("checks = 1", "table of checks"),
            ("not toml [", "expected"),
        ] {
            let e = parse(text, Origin::Person).unwrap_err();
            assert!(e.contains(expect), "{text:?} gave {e:?}");
        }
    }

    #[test]
    fn a_project_without_the_table_has_empty_layers_and_a_present_one_is_read() {
        use crate::infra::project::{FILE, PERSON_FILE, find};
        let dir = std::env::temp_dir().join(format!("chidev-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".chidev")).unwrap();
        std::fs::write(dir.join(FILE), "").unwrap();
        let layers = load(&find(&dir).unwrap()).unwrap();
        assert_eq!(layers.len(), 2);
        assert!(layers.iter().all(|l| l.entries.is_empty()));
        std::fs::write(
            dir.join(PERSON_FILE),
            "[checks.\"context.\"]\nlevel = \"off\"\n",
        )
        .unwrap();
        std::fs::write(dir.join(FILE), "[checks]\nx = 1\n").unwrap();
        let error = load(&find(&dir).unwrap()).unwrap_err();
        assert!(error.path.ends_with(FILE), "{error}");
        std::fs::write(dir.join(FILE), "").unwrap();
        let layers = load(&find(&dir).unwrap()).unwrap();
        assert_eq!(layers[0].origin, Origin::Project);
        assert_eq!(layers[1].origin, Origin::Person);
        assert_eq!(layers[1].entries.len(), 1);
        std::fs::write(dir.join(PERSON_FILE), "[checks]\nx = 1\n").unwrap();
        let error = load(&find(&dir).unwrap()).unwrap_err();
        assert!(error.path.ends_with("chidev.local.toml"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
