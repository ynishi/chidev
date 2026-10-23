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

use chidev_core::app::App;
use chidev_core::infra::ids::UuidV7;
use chidev_core::infra::memory::MemoryStore;
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
    /// Runs the operations in FILE (or stdin), one JSON object per line, against one project kept in memory, and
    /// prints one JSON result per operation. The project is gone when the run ends.
    Exec {
        /// The JSON lines to run; stdin when not given.
        file: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Exec { file } => {
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
            let mut app = App::new(MemoryStore::default(), UuidV7);
            let mut out = io::stdout().lock();
            match ops::run(&mut app, reader, &mut out) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(e) => {
                    let _ = out.flush();
                    eprintln!("chidev: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
