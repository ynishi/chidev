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
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use chidev_core::app::{App, Registry};
use chidev_core::domain::check::builtin;
use chidev_core::infra::accept;
use chidev_core::infra::clock::SystemClock;
use chidev_core::infra::external::{self, ProcessExecutor};
use chidev_core::infra::ids::UuidV7;
use chidev_core::infra::judged;
use chidev_core::infra::policy;
use chidev_core::infra::project;
use chidev_core::infra::runner;
use chidev_core::infra::sqlite::SqliteStore;
use clap::{Args, Parser, Subcommand};

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
        #[command(flatten)]
        store: StoreArg,
    },
    /// Runs every check on the project kept in the store as it is, and prints the result of the operation `check`
    /// (a JSON object: the `findings`, and `outside` when RULE or `--context` left some out). Nothing is kept. Each
    /// RULE is a rule id (`examples.failed`) or a prefix of whole segments (`examples`); the findings are those of
    /// the rules selected, all when none is given. A RULE that matches no check is refused. The exit code is 0 when
    /// no finding is at `error`, 1 when one is or the operation was refused (an unknown rule or context), and 2
    /// when it could not be run (bad arguments, no project, no store).
    Check {
        /// The context to narrow the findings to.
        #[arg(long, value_name = "C")]
        context: Option<String>,
        #[command(flatten)]
        store: StoreArg,
        /// The rules to report, each an id or a prefix of whole segments.
        #[arg(value_name = "RULE")]
        rules: Vec<String>,
    },
    /// Measures, in a copy of the project, and keeps what was observed: prints the result of the operation `measure`
    /// (a JSON object). It takes as long as the project's build, tests, script or program take. The exit code is 0
    /// when the run was recorded and did not fail, 1 when it was recorded and failed or the operation was refused
    /// (no Pin, an unknown name, no `[measure]` table), and 2 when it could not be run (bad arguments, no project,
    /// no store).
    Measure {
        #[command(subcommand)]
        measure: Measure,
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
    /// Runs the acceptance examples in DIR against a program, one after another, and prints a line for each and
    /// a count (`ok`, or `fail` with every reason). An example is a name with files `<name>.args` (the arguments,
    /// split on white space, after those of the command), `<name>.stdin`, `<name>.expected` (the standard output,
    /// then a last line `#exit <code>`) and, optionally, `<name>.stderr` (the standard error, compared when it is
    /// there). The program runs without a shell, from the current directory, in a process group that is killed when
    /// it ends. The exit code is 0 when every example passed, 1 when one failed, and 2 when nothing could be run.
    ///
    /// The examples a Harness of kind `examples` holds in the model are run by `measure` of kind `examples`, which
    /// keeps the run.
    Accept {
        /// The most seconds one example may take (1 to 86400, a day); a program that is still running is killed.
        #[arg(
            long,
            value_name = "SECONDS",
            default_value_t = 10,
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        timeout: u64,
        /// The directory of the examples, relative to the current directory.
        dir: PathBuf,
        /// The program to run and its first arguments, after `--`.
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<String>,
    },
}

/// Where the project is kept, for the commands that run on it.
#[derive(Debug, Args)]
struct StoreArg {
    /// The database that keeps the project, relative to the current directory. Without it, the project's settings
    /// say (`[store]` in `chidev.toml`), and else it is `.chidev/chidev.db` under the project's root. `check` and
    /// `measure` open a database that is there and stop with status 2 when it is not; `exec` makes it.
    #[arg(long, value_name = "PATH")]
    store: Option<PathBuf>,
}

/// What `chidev measure` measures: a subcommand of each kind, so that the asset the kind takes is the only one that
/// can be given.
#[derive(Debug, Subcommand)]
enum Measure {
    /// Runs the code the Pin of an Interface accepted against the Harnesses as the model holds them, once per
    /// mutant (`measure` of kind `mutate`).
    Mutate {
        /// The Context.
        #[arg(long, value_name = "C")]
        context: String,
        /// The Interface whose pinned code is mutated.
        #[arg(long, value_name = "I")]
        interface: String,
        #[command(flatten)]
        store: StoreArg,
    },
    /// Builds the program from the code of the Pins of the Interfaces a sim Harness cites and runs its script once
    /// per case (`measure` of kind `simulate`).
    Simulate {
        /// The Context.
        #[arg(long, value_name = "C")]
        context: String,
        /// The sim Harness.
        #[arg(long, value_name = "H")]
        harness: String,
        #[command(flatten)]
        store: StoreArg,
    },
    /// Builds the program the same way and runs each example of a Harness of examples against it (`measure` of kind
    /// `examples`).
    Examples {
        /// The Context.
        #[arg(long, value_name = "C")]
        context: String,
        /// The Harness of examples.
        #[arg(long, value_name = "H")]
        harness: String,
        #[command(flatten)]
        store: StoreArg,
    },
}

impl Measure {
    /// The store, the Context, and what to measure in it.
    fn parts(self) -> (StoreArg, String, ops::Measuring) {
        match self {
            Measure::Mutate {
                context,
                interface,
                store,
            } => (store, context, ops::Measuring::Mutate { interface }),
            Measure::Simulate {
                context,
                harness,
                store,
            } => (store, context, ops::Measuring::Simulate { harness }),
            Measure::Examples {
                context,
                harness,
                store,
            } => (store, context, ops::Measuring::Examples { harness }),
        }
    }
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

/// Runs the examples of the directory `dir` against `command` and prints a line for each and a count. A failure to
/// run at all is a message on standard error, the exit code 2 and nothing on standard output.
async fn accept(dir: PathBuf, command: Vec<String>, timeout: u64) -> Result<bool, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    let timeout = Duration::from_secs(timeout);
    let results = accept::run_all(&dir, &command, &cwd, timeout)
        .await
        .map_err(|e| e.to_string())?;
    let passed = results.iter().all(|(_, verdict)| verdict.is_ok());
    let mut out = io::stdout().lock();
    out.write_all(accept::lines(&results).as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("cannot write the report: {e}"))?;
    Ok(passed)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Accept {
            timeout,
            dir,
            command,
        } => match accept(dir, command, timeout).await {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(e) => {
                eprintln!("chidev accept: {e}");
                ExitCode::from(2)
            }
        },
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
            with_app(store, Opening::Create, async move |app, root| {
                let mut out = io::stdout().lock();
                let done = ops::run(app, root, reader, &mut out).await;
                exit_code(done, &mut out)
            })
            .await
        }
        Command::Check {
            context,
            store,
            rules,
        } => {
            with_app(store, Opening::Existing, async move |app, root| {
                let mut out = io::stdout().lock();
                let done = ops::check(app, root, context, rules, &mut out).await;
                exit_code(done, &mut out)
            })
            .await
        }
        Command::Measure { measure } => {
            let (store, context, what) = measure.parts();
            with_app(store, Opening::Existing, async move |app, root| {
                let mut out = io::stdout().lock();
                let done = ops::measure(app, root, context, what, &mut out).await;
                exit_code(done, &mut out)
            })
            .await
        }
    }
}

/// How a command opens the store of the project.
#[derive(Debug, Clone, Copy)]
enum Opening {
    /// Makes the store when it is not there: the first operation of a project is `init`, which `exec` runs.
    Create,
    /// Opens a store that is there and refuses one that is not, so that a mistyped path or a project never started
    /// does not leave an empty store behind.
    Existing,
}

/// The use cases over the project kept in the store, as every command that runs on a project has them.
type ProjectApp = App<SqliteStore, UuidV7, ProcessExecutor>;

/// Runs `body` on the project the command runs in, and returns its exit code. The one place that sets a project up:
/// the project's root is the nearest directory, from the one the command runs in, that has a `chidev.toml`; the
/// settings, the judged and external rules and the measure table are read from there, and so are the files a
/// `source_file` or a `code_file` names. No root, any of them being unreadable, or the store not opening stops the
/// run with status 2 before `body` runs. The store is closed after `body`, always and explicitly, never on drop (a
/// close that fails is status 2 too).
async fn with_app(
    store: StoreArg,
    opening: Opening,
    body: impl AsyncFnOnce(&mut ProjectApp, &Path) -> ExitCode,
) -> ExitCode {
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
    let store = store.store.unwrap_or_else(|| project.store().to_path_buf());
    let registry = match (|| -> Result<_, String> {
        let layers = policy::load(&project).map_err(|e| format!("policy: {e}"))?;
        let rules = judged::load(root).map_err(|e| format!("rules: {e}"))?;
        let (external_rules, executor) =
            external::load(&project).map_err(|e| format!("external: {e}"))?;
        let registry = Registry::with_external(builtin::all(), rules, external_rules, &layers)
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
    let sqlite = match opening {
        Opening::Create => SqliteStore::open(&store).await,
        Opening::Existing => SqliteStore::open_existing(&store).await,
    };
    let sqlite = match sqlite {
        Ok(s) => s,
        Err(e) => {
            eprintln!("chidev: {}: {e}", store.display());
            return ExitCode::from(2);
        }
    };
    let (registry, executor, measure) = registry;
    let mut app = App::with_registry(sqlite, UuidV7, registry, SystemClock).with_external(executor);
    if let Some(measure) = measure {
        // one table answers both: how to measure, and how to find the Items of an Interface
        app = app.with_items(measure.clone()).with_runner(measure);
    }
    let code = body(&mut app, root).await;
    if let Err(e) = app.into_store().close().await {
        eprintln!("chidev: {}: {e}", store.display());
        return ExitCode::from(2);
    }
    code
}

/// The exit code of a command that wrote its result to `out`: 0 when it succeeded, 1 when it did not, and 2 when it
/// could not read its input or write its result.
fn exit_code(done: io::Result<bool>, out: &mut impl Write) -> ExitCode {
    match done {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            let _ = out.flush();
            eprintln!("chidev: {e}");
            ExitCode::from(2)
        }
    }
}
