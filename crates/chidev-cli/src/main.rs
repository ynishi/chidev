//! The `chidev` command: parses arguments, calls
//! `chidev-core`, prints the result.
//!
//! The argument parser's definition (clap's derive types) is the only
//! description of the commands: help, the command list and validation come
//! from it, not from a second hand-kept table. Structured input is read from a
//! file argument, not pasted into the command line.

mod ops;
mod schema;

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use chidev_core::app::{App, Registry};
use chidev_core::domain::check::builtin;
use chidev_core::infra::clock::SystemClock;
use chidev_core::infra::external;
use chidev_core::infra::ids::UuidV7;
use chidev_core::infra::judged;
use chidev_core::infra::policy;
use chidev_core::infra::project;
use chidev_core::infra::runner;
use chidev_core::infra::sqlite::SqliteStore;
use clap::{Parser, Subcommand};

/// Contract, Interface and Harness as the assets of a codebase; the code is a cache pinned to them.
#[derive(Debug, Parser)]
#[command(name = "chidev", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Runs the operations in FILE (or stdin), one JSON object per line, against the project kept in the store, and
    /// prints one JSON result per operation. What the run changes is kept for the next one.
    Exec {
        /// The JSON lines to run; stdin when not given.
        file: Option<PathBuf>,
        /// The database that keeps the project, relative to the current directory. Without it, the project's
        /// settings say (`[store]` in `chidev.toml`), and else it is `.chidev/chidev.db` under the project's root.
        #[arg(long, value_name = "PATH")]
        store: Option<PathBuf>,
    },
    /// Prints a JSON Schema (2020-12): `ops` for a line of `exec`, `view` for the `view` an operation of that name
    /// returns. Without a name, lists the schemas. A schema says what a document must look like to be read; a rule
    /// that sets fields against each other is not in it, and is named when an operation breaks it.
    Schema {
        /// Which schema to print.
        #[arg(value_enum)]
        name: Option<schema::Name>,
    },
    /// The providers that come with chidev: programs that speak a protocol on standard input and output, which a
    /// project names in the `[measure]` table of `chidev.toml` (`mutants`, `items`) like any provider of its own.
    Provider {
        #[command(subcommand)]
        provider: Provider,
    },
}

#[derive(Debug, Subcommand)]
enum Provider {
    /// Makes the mutants of a Rust source: reads one `chidev-mutants 1` request (JSON) on standard input and writes
    /// the reply (JSON) on standard output. Pure: it reads no file and runs nothing.
    MutantsRust,
    /// Finds the Items of a Rust source: reads one `chidev-items 1` request (JSON) on standard input and writes the
    /// reply (JSON) on standard output, a key and the lines of each declaration. Pure: it reads no file and runs
    /// nothing.
    ItemsRust,
}

/// Runs the provider `name`, whose protocol is `handle`: the protocol is the standard input and output, so a failure
/// is a message on standard error and an exit code, never a half reply.
fn serve<E: std::fmt::Display>(
    name: &str,
    handle: impl FnOnce(&[u8]) -> Result<Vec<u8>, E>,
) -> ExitCode {
    let mut request = Vec::new();
    if let Err(e) = io::stdin().lock().read_to_end(&mut request) {
        eprintln!("chidev provider {name}: cannot read the request: {e}");
        return ExitCode::from(2);
    }
    match handle(&request) {
        Ok(reply) => {
            let mut out = io::stdout().lock();
            match out.write_all(&reply).and_then(|()| out.flush()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("chidev provider {name}: cannot write the reply: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Err(e) => {
            eprintln!("chidev provider {name}: {e}");
            ExitCode::from(2)
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Schema { name } => {
            // a document to read, not a line of a stream: indented
            let printed = match name {
                Some(name) => serde_json::to_string_pretty(&name.schema()),
                None => serde_json::to_string_pretty(&schema::list()),
            };
            let printed = match printed {
                Ok(printed) => printed,
                Err(e) => {
                    eprintln!("chidev schema: cannot write the schema: {e}");
                    return ExitCode::from(2);
                }
            };
            match writeln!(io::stdout().lock(), "{printed}") {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("chidev schema: cannot write the schema: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Command::Provider {
            provider: Provider::MutantsRust,
        } => serve("mutants-rust", chidev_provider_rust::handle),
        Command::Provider {
            provider: Provider::ItemsRust,
        } => serve("items-rust", chidev_provider_rust::items::handle),
        Command::Exec { file, store } => {
            let reader: Box<dyn BufRead> = match file {
                Some(path) => match File::open(&path) {
                    Ok(f) => Box::new(BufReader::new(f)),
                    Err(e) => {
                        eprintln!("chidev: {}: {e}", path.display());
                        return ExitCode::from(2);
                    }
                },
                None => Box::new(BufReader::new(io::stdin())),
            };
            // The project's root is the nearest directory, from the one the command runs in, that has a `chidev.toml`;
            // the settings and the judged rules are read from there, and so are the files a `source_file` or a
            // `code_file` names. No root, or either being unreadable, stops the run before any operation.
            let project = match std::env::current_dir()
                .map_err(|e| e.to_string())
                .and_then(|cwd| project::find(&cwd).map_err(|e| e.to_string()))
            {
                Ok(project) => project,
                Err(e) => {
                    eprintln!("chidev: {e}");
                    return ExitCode::from(2);
                }
            };
            let root = project.root();
            // the flag is relative to the current directory, as a path on the command line is; the setting is not
            let store = store.unwrap_or_else(|| project.store().to_path_buf());
            let registry = match (|| -> Result<_, String> {
                let layers = policy::load(&project).map_err(|e| format!("policy: {e}"))?;
                let rules = judged::load(root).map_err(|e| format!("rules: {e}"))?;
                let (external_rules, executor) =
                    external::load(&project).map_err(|e| format!("external: {e}"))?;
                let registry =
                    Registry::with_external(builtin::all(), rules, external_rules, &layers)
                        .map_err(|e| format!("policy: {e}"))?;
                let measure = runner::load(&project).map_err(|e| format!("measure: {e}"))?;
                Ok((registry, executor, measure))
            })() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("chidev: {e}");
                    return ExitCode::from(2);
                }
            };
            let sqlite = match SqliteStore::open(&store).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("chidev: {}: {e}", store.display());
                    return ExitCode::from(2);
                }
            };
            let (registry, executor, measure) = registry;
            let mut app =
                App::with_registry(sqlite, UuidV7, registry, SystemClock).with_external(executor);
            if let Some(measure) = measure {
                // one table answers both: how to measure, and how to find the Items of an Interface
                app = app.with_items(measure.clone()).with_runner(measure);
            }
            let mut out = io::stdout().lock();
            let code = match ops::run(&mut app, root, reader, &mut out).await {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(e) => {
                    let _ = out.flush();
                    eprintln!("chidev: {e}");
                    ExitCode::from(2)
                }
            };
            if let Err(e) = app.into_store().close().await {
                eprintln!("chidev: {}: {e}", store.display());
                return ExitCode::from(2);
            }
            code
        }
    }
}
