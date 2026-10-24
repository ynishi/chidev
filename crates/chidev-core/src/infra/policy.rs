//! The policy files, read into the typed layers the domain resolves: `checks.toml` in the project's root (the
//! project's choices, kept with it) and `.chidev/checks.local.toml` (the person's own, not kept with the project). A
//! missing file is an empty layer. The shape of a file:
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
//! The file is parsed here, at the edge; what a key may name and what a mandatory check may be set to are the
//! domain's rules ([`Policy::resolve`](crate::domain::check::Policy::resolve)), so a file is checked against the
//! checks that exist when the registry is built.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::check::policy::ParamValue;
use crate::domain::check::{Layer, Level, Origin, Selector, Setting};

/// The project's policy file, relative to the project's root.
pub const PROJECT_FILE: &str = "checks.toml";

/// The person's policy file, relative to the project's root.
pub const PERSON_FILE: &str = ".chidev/checks.local.toml";

/// Why a policy file cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyFileError {
    /// The file exists and cannot be read.
    #[error("{}: {message}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What the system said.
        message: String,
    },
    /// The file is not a policy.
    #[error("{}: {message}", path.display())]
    Invalid {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
}

/// The layers of the policy for the project rooted at `root`: the project's, then the person's. A file that is not
/// there adds an empty layer.
pub fn load(root: &Path) -> Result<Vec<Layer>, PolicyFileError> {
    Ok(vec![
        read(&root.join(PROJECT_FILE), Origin::Project)?,
        read(&root.join(PERSON_FILE), Origin::Person)?,
    ])
}

fn read(path: &Path, origin: Origin) -> Result<Layer, PolicyFileError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, origin).map_err(|message| PolicyFileError::Invalid {
            path: path.to_path_buf(),
            message,
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Layer {
            origin,
            entries: Vec::new(),
        }),
        Err(e) => Err(PolicyFileError::Read {
            path: path.to_path_buf(),
            message: e.to_string(),
        }),
    }
}

/// The layer `text` is, for `origin`.
pub fn parse(text: &str, origin: Origin) -> Result<Layer, String> {
    let doc: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut entries = Vec::new();
    for (key, value) in &doc {
        // Project registrations are validated by the external adapter loader. A person
        // may override levels here, but cannot register programs in their policy layer.
        if key == "external" && origin == Origin::Project {
            if !value.is_table() {
                return Err("`external` is a table of rule registrations".into());
            }
            continue;
        }
        if key != "checks" {
            return Err(format!(
                "`{key}` is not a section of a policy; there is `checks`"
            ));
        }
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
    fn a_missing_file_is_an_empty_layer_and_a_present_one_is_read() {
        let dir = std::env::temp_dir().join(format!("chidev-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".chidev")).unwrap();
        let layers = load(&dir).unwrap();
        assert_eq!(layers.len(), 2);
        assert!(layers.iter().all(|l| l.entries.is_empty()));
        std::fs::write(
            dir.join(PERSON_FILE),
            "[checks.\"context.\"]\nlevel = \"off\"\n",
        )
        .unwrap();
        std::fs::write(dir.join(PROJECT_FILE), "[checks\n").unwrap();
        assert!(matches!(load(&dir), Err(PolicyFileError::Invalid { .. })));
        std::fs::remove_file(dir.join(PROJECT_FILE)).unwrap();
        let layers = load(&dir).unwrap();
        assert_eq!(layers[1].origin, Origin::Person);
        assert_eq!(layers[1].entries.len(), 1);
    }
}
