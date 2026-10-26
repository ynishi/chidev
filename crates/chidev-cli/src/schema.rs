//! `chidev schema`: the JSON Schema of what an agent writes to `chidev exec`, of the view of the model it reads
//! back, and of the request an external check is sent and the reply it gives, derived from the types that read and
//! write them.
//!
//! A schema states what a document must look like to be read. It is not the whole of what is refused: a rule that
//! sets fields against each other (exactly one of `source` and `source_file`) is not in it, and an operation that
//! breaks one is refused by the rule's name when it runs.

use chidev_core::app::ports::external::{ExternalCall, ExternalReply, REQUEST_PROTOCOL};
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
    /// The request an external check is sent on its stdin.
    ExternalCall,
    /// The reply an external check gives on its stdout.
    ExternalReply,
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
            Name::ExternalCall => format!("chidev-external-call {REQUEST_PROTOCOL}"),
            Name::ExternalReply => format!("chidev-external-reply {REQUEST_PROTOCOL}"),
        }
    }

    /// The identifier of the schema, which does not depend on where it is read from.
    fn id(self) -> String {
        match self {
            Name::Ops => format!("urn:chidev:schema:ops:{EXEC_PROTOCOL}"),
            Name::View => format!("urn:chidev:schema:view:{VIEW_VERSION}"),
            Name::ExternalCall => format!("urn:chidev:schema:external-call:{REQUEST_PROTOCOL}"),
            Name::ExternalReply => format!("urn:chidev:schema:external-reply:{REQUEST_PROTOCOL}"),
        }
    }

    /// What the schema describes, in a line.
    fn about(self) -> &'static str {
        match self {
            Name::Ops => "one line chidev exec reads: an operation",
            Name::View => "the view of an operation view: the part of the model that was asked for",
            Name::ExternalCall => "the request an external check is sent on its stdin",
            Name::ExternalReply => "the reply an external check gives on its stdout",
        }
    }

    /// The schema (JSON Schema 2020-12), with the title and the identifier above. What is read (an operation, a
    /// reply) is described as it is read, and what is written (the view, a request) as it is written: a section that
    /// was not asked for is left out of the view, not `null`.
    pub fn schema(self) -> Value {
        let mut schema = match self {
            Name::Ops => crate::ops::input_schema(),
            Name::View => SchemaSettings::draft2020_12()
                .for_serialize()
                .into_generator()
                .into_root_schema_for::<ModelSlice>(),
            Name::ExternalCall => SchemaSettings::draft2020_12()
                .for_serialize()
                .into_generator()
                .into_root_schema_for::<ExternalCall>(),
            Name::ExternalReply => SchemaSettings::draft2020_12()
                .for_deserialize()
                .into_generator()
                .into_root_schema_for::<ExternalReply>(),
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
