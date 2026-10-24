//! The `chidev` command: parses arguments, calls
//! `chidev-core`, prints the result.
//!
//! The argument parser's definition (clap's derive types) is the only
//! description of the commands: help, the command list and validation come
//! from it, not from a second hand-kept table. Structured input is read from a
//! file argument, not pasted into the command line.

mod ops;

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use chidev_core::app::{App, Registry};
use chidev_core::infra::ids::UuidV7;
use chidev_core::infra::judged;
use chidev_core::infra::policy;
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
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
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
                        Registry::builtin_with_judged(rules, &layers)
                            .map_err(|e| format!("policy: {e}"))
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
            let mut app = App::with_registry(sqlite, UuidV7, registry);
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
