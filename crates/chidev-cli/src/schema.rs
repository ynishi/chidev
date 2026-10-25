//! `chidev schema`: the JSON Schema of what an agent writes to `chidev exec` and of the view of the model it reads
//! back, derived from the types that read and write them.
//!
//! A schema states what a document must look like to be read. It is not the whole of what is refused: a rule that
//! sets fields against each other (exactly one of `source` and `source_file`) is not in it, and an operation that
//! breaks one is refused by the rule's name when it runs.

use chidev_core::app::{ModelSlice, VIEW_VERSION};
use clap::ValueEnum;
use schemars::generate::SchemaSettings;
use serde::Serialize;
use serde_json::{Value, json};

/// The version of the protocol `chidev exec` speaks: the operations it reads and the shape of the lines it prints.
const EXEC_PROTOCOL: u32 = 1;

/// A schema `chidev schema` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Name {
    /// One line `chidev exec` reads: an operation.
    Ops,
    /// The `view` an operation `view` returns: the part of the model that was asked for.
    View,
}

impl Name {
    /// The name as it is typed on the command line.
    fn word(self) -> String {
        self.to_possible_value()
            .map(|v| v.get_name().to_string())
            .unwrap_or_default()
    }

    /// The protocol and its version: the name a reader checks the version by.
    fn title(self) -> String {
        match self {
            Name::Ops => format!("chidev-exec {EXEC_PROTOCOL}"),
            Name::View => format!("chidev-view {VIEW_VERSION}"),
        }
    }

    /// The identifier of the schema, which does not depend on where it is read from.
    fn id(self) -> String {
        match self {
            Name::Ops => format!("urn:chidev:schema:ops:{EXEC_PROTOCOL}"),
            Name::View => format!("urn:chidev:schema:view:{VIEW_VERSION}"),
        }
    }

    /// What the schema describes, in a line.
    fn about(self) -> &'static str {
        match self {
            Name::Ops => "one line chidev exec reads: an operation",
            Name::View => "the view of an operation view: the part of the model that was asked for",
        }
    }

    /// The schema (JSON Schema 2020-12), with the title and the identifier above. What is read (an operation) is
    /// described as it is read, and what is written (the view) as it is written: a section that was not asked for
    /// is left out of the view, not `null`.
    pub fn schema(self) -> Value {
        let mut schema = match self {
            Name::Ops => crate::ops::input_schema(),
            Name::View => SchemaSettings::draft2020_12()
                .for_serialize()
                .into_generator()
                .into_root_schema_for::<ModelSlice>(),
        };
        schema.insert("title".to_string(), json!(self.title()));
        schema.insert("$id".to_string(), json!(self.id()));
        schema.to_value()
    }
}

/// The schemas there are, to choose from: what `chidev schema` prints without a name.
#[derive(Debug, Serialize)]
pub struct SchemaList {
    schemas: Vec<SchemaEntry>,
}

/// One schema of the list.
#[derive(Debug, Serialize)]
struct SchemaEntry {
    name: String,
    title: String,
    #[serde(rename = "$id")]
    id: String,
    about: &'static str,
}

/// The schemas there are, in the order clap lists the names.
pub fn list() -> SchemaList {
    SchemaList {
        schemas: Name::value_variants()
            .iter()
            .map(|n| SchemaEntry {
                name: n.word(),
                title: n.title(),
                id: n.id(),
                about: n.about(),
            })
            .collect(),
    }
}
