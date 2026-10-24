//! The `chidev` command: parses arguments, calls
//! `chidev-core`, prints the result.
//!
//! The argument parser's definition (clap's derive types) is the only
//! description of the commands: help, the command list and validation come
//! from it, not from a second hand-kept table. Structured input is read from a
//! file argument, not pasted into the command line.

mod ops;

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
use chidev_core::infra::runner;
use chidev_core::infra::sqlite::SqliteStore;
use clap::{Parser, Subcommand};

/// Where the project is kept when `--store` is not given, under the current directory.
const DEFAULT_STORE: &str = ".chidev/chidev.db";

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
        /// The database that keeps the project.
        #[arg(long, value_name = "PATH", default_value = DEFAULT_STORE)]
        store: PathBuf,
    },
    /// The providers that come with chidev: programs that speak a protocol on standard input and output, which a
    /// project names in the `[measure]` table of `checks.toml` like any provider of its own.
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
}

/// Runs the provider `MutantsRust`: the protocol is the standard input and output, so a failure is a message on
/// standard error and an exit code, never a half reply.
fn mutants_rust() -> ExitCode {
    let mut request = Vec::new();
    if let Err(e) = io::stdin().lock().read_to_end(&mut request) {
        eprintln!("chidev provider mutants-rust: cannot read the request: {e}");
        return ExitCode::from(2);
    }
    match chidev_provider_rust::handle(&request) {
        Ok(reply) => {
            let mut out = io::stdout().lock();
            match out.write_all(&reply).and_then(|()| out.flush()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("chidev provider mutants-rust: cannot write the reply: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Err(e) => {
            eprintln!("chidev provider mutants-rust: {e}");
            ExitCode::from(2)
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Provider {
            provider: Provider::MutantsRust,
        } => mutants_rust(),
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
            // The project's root is the directory the command runs in; its policy files and the judged rules are read
            // from there. Either being unreadable stops the run before any operation.
            let registry =
                match std::env::current_dir()
                    .map_err(|e| e.to_string())
                    .and_then(|root| {
                        let layers = policy::load(&root).map_err(|e| format!("policy: {e}"))?;
                        let rules = judged::load(&root).map_err(|e| format!("rules: {e}"))?;
                        let (external_rules, executor) =
                            external::load(&root).map_err(|e| format!("external: {e}"))?;
                        let registry =
                            Registry::with_external(builtin::all(), rules, external_rules, &layers)
                                .map_err(|e| format!("policy: {e}"))?;
                        let measure = runner::load(&root).map_err(|e| format!("measure: {e}"))?;
                        Ok((registry, executor, measure))
                    }) {
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
                app = app.with_runner(measure);
            }
            let mut out = io::stdout().lock();
            let code = match ops::run(&mut app, reader, &mut out).await {
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
