//! Running a sim Harness: the protocol `chidev-sim 2`, language-independent.
//!
//! chidev copies the project (as a mutation run does), writes the code the Pins of the Interfaces the Harness builds
//! accepted and the script over the copy, and runs the project's build command. It then starts the script once per
//! case:
//!
//! ```text
//! <interpreter...> <script> <case> --out <dir> -- <program argv...>
//! ```
//!
//! `<case>` is `seed-<n>` or `regress-<name>`, the working directory is the copy, and `<program argv...>` is the
//! configured `program` with `{tree}` made the root of the copy (`rerun-tree` in the second copy) and `{run}` the
//! directory of the run. The script drives the program as a black box (standard input and output, arguments) and
//! writes two files into `<dir>`: `schedule`, the choices it made for this case from its seed, and `trace`, what the
//! program did under them. chidev takes their SHA-256 digests; the script does not report them. On standard output
//! the script prints, one per line:
//!
//! ```text
//! FINDING <case> step <k> <contract>:<label>: <text>   a promise of the Clause did not hold after step k (k from 1)
//! UNREACHED <contract>:<label>: <text>                 this case never reached the situation the Clause needs
//! UNDECIDED <text>                                     the script could not decide something (counted, never failed)
//! CANNOT_START <text>                                  the script could not start the program, so it drove nothing
//! sim: findings=<n>                                    last line; n is the number of FINDING lines
//! ```
//!
//! `CANNOT_START` is the one line a script prints when starting the program failed (a program that is not there, a
//! file that cannot be run): the script, which starts the program, is the only one who sees it. The report then
//! holds that line and the summary `sim: findings=0` and nothing else; a `FINDING`, `UNREACHED` or `UNDECIDED` line
//! beside it is a protocol failure, for they would tell of a run that did not happen. The case is recorded as
//! `cannot_start`: it has no findings and says nothing of the Clauses. When *every* case that was run could not
//! start the program, the run is a failure of kind `no_start` and the second runs are not made.
//!
//! A Clause is named `<contract>:<label>`: the Contract's name (as chidev knows it, without white space) and the
//! label (`C<n>`) of the Clause in it. Labels are unique only within a Contract, so a sim Harness that checks
//! Clauses of several Contracts names each by its Contract. chidev resolves each name to the Clause's id when it
//! records the run, against the Clauses the sweep Case cites; the record holds ids, not names.
//!
//! Exit 0 or 1 (1 when there are findings) means the report is to be read. A name of a Contract the sweep Case
//! cites nothing of, or of a Clause of one that it does not cite, a name that is not `<contract>:<label>`, a
//! `FINDING` for another case, a line of none of these forms, a missing or repeated summary, a summary that
//! disagrees with the lines, or a normal case that wrote no `schedule` and `trace` is a protocol failure of the run.
//! Any other exit code, or a script that runs past the time allowed for a case, is recorded for that case and the
//! run goes on.
//!
//! After the cases, `seed-0` and up to `sample` cases that had findings are run once more in a second, fresh copy,
//! and their digests are compared with the first run's: a different schedule is the script's fault, a different
//! trace the program's. That check is chidev's, never the script's own word.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::super::process::{self, Capture, Over, Pipe};
use super::config::SimConfig;
use super::{Command, ERROR_LIMIT, Job, ProcessRunner, TAIL, config_error, io, millis, provider};
use crate::app::ports::measure::{InterfaceText, RunnerError, SimulateReport, SimulateRequest};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::ClauseId;
use crate::domain::vo::measure::{FailureKind, MeasureFailure, StepEvidence};
use crate::domain::vo::sim::{
    CaseExit, CaseFinding, CaseRun, CaseUnreached, CitedClause, ClauseName, Rerun, SimCase,
    reproduction, rerun_cases, resolve,
};

/// The longest report a script may print.
const REPORT_LIMIT: usize = 1024 * 1024;

/// What a script said of one case.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Reported {
    pub findings: Vec<CaseFinding>,
    pub unreached: Vec<CaseUnreached>,
    pub undecided: Vec<String>,
    /// Why the script could not start the program (`CANNOT_START <text>`); then it reported nothing else.
    pub cannot_start: Option<String>,
}

fn shown(line: &str) -> String {
    const MAX: usize = 160;
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        let cut: String = line.chars().take(MAX).collect();
        format!("{cut}...")
    }
}

/// `<contract>:<label>: <text>`: the name of a Clause the sweep cites, resolved to the Clause's id, and the text.
/// The name is the first word and ends in a colon; a Contract's name has no white space.
fn named<'a>(rest: &'a str, cited: &[CitedClause]) -> Result<(ClauseId, &'a str), String> {
    let shape = || "no `<contract>:<label>: <text>`".to_string();
    let (word, text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let name = word.strip_suffix(':').ok_or_else(shape)?;
    let name = ClauseName::try_from(name)
        .map_err(|_| format!("`{name}` is not a clause name (`<contract>:C<n>`)"))?;
    let id = resolve(cited, &name).map_err(|e| e.to_string())?;
    let text = text.trim();
    if text.is_empty() {
        return Err("the text after the clause name is empty".to_string());
    }
    Ok((id, text))
}

/// `<case> step <k> <contract>:<label>: <text>` of a `FINDING` line.
fn finding_of(rest: &str, case: &SimCase, cited: &[CitedClause]) -> Result<CaseFinding, String> {
    let (named_case, after) = rest
        .split_once(' ')
        .ok_or_else(|| "no `<case> step <k> <contract>:<label>: <text>`".to_string())?;
    if named_case != case.to_string() {
        return Err(format!(
            "the finding is of `{named_case}`, and this run is of `{case}`"
        ));
    }
    let after = after
        .strip_prefix("step ")
        .ok_or_else(|| "no `step <k>` after the case".to_string())?;
    let (step, after) = after
        .split_once(' ')
        .ok_or_else(|| "no clause after `step <k>`".to_string())?;
    let step: u32 = step
        .parse()
        .ok()
        .filter(|k| *k >= 1)
        .ok_or_else(|| format!("`{step}` is not a step (a number from 1)"))?;
    let (clause, text) = named(after, cited)?;
    Ok(CaseFinding {
        step,
        clause,
        text: text.to_string(),
    })
}

/// Reads the report of the script run for `case`. `cited` are the Clauses the sweep cites, by the names a script
/// writes them; each name in the report is resolved to the Clause's id. Any line that is not one of the forms, a
/// name of a contract or a clause the sweep does not cite, a finding of another case, no summary or two, a summary
/// that is not the last line, and a summary that counts other than the lines it follows are errors; the message
/// says which line.
pub(super) fn parse_report(
    case: &SimCase,
    cited: &[CitedClause],
    text: &str,
) -> Result<Reported, String> {
    let mut out = Reported::default();
    let mut summary: Option<(usize, u32)> = None;
    let mut any = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        any = true;
        let at = n + 1;
        let bad = |why: String| format!("line {at} `{}`: {why}", shown(line));
        if summary.is_some() {
            return Err(bad("a line after the `sim:` summary".to_string()));
        }
        if let Some(rest) = line.strip_prefix("FINDING ") {
            out.findings
                .push(finding_of(rest, case, cited).map_err(bad)?);
        } else if let Some(rest) = line.strip_prefix("UNREACHED ") {
            let (clause, text) = named(rest, cited).map_err(bad)?;
            out.unreached.push(CaseUnreached {
                clause,
                text: text.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix("UNDECIDED ") {
            if rest.trim().is_empty() {
                return Err(bad("an empty reason".to_string()));
            }
            out.undecided.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("CANNOT_START ") {
            if rest.trim().is_empty() {
                return Err(bad("an empty reason".to_string()));
            }
            if out.cannot_start.is_some() {
                return Err(bad("`CANNOT_START` twice".to_string()));
            }
            out.cannot_start = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("sim: findings=") {
            let count: u32 = rest
                .trim()
                .parse()
                .map_err(|_| bad(format!("`{}` is not a count", rest.trim())))?;
            summary = Some((at, count));
        } else {
            return Err(bad("none of the forms of chidev-sim 2".to_string()));
        }
    }
    match summary {
        None => Err(if any {
            "the report has no `sim: findings=<n>` as its last line".to_string()
        } else {
            "the report is empty; its last line is `sim: findings=<n>`".to_string()
        }),
        Some((at, n)) if usize::try_from(n).ok() != Some(out.findings.len()) => Err(format!(
            "line {at}: the summary says findings={n} and the report has {}",
            out.findings.len()
        )),
        // A script that never drove the program checked nothing: a report that also names findings, unreached
        // Clauses or undecided things would be telling of a run that did not happen.
        Some(_)
            if out.cannot_start.is_some()
                && !(out.findings.is_empty()
                    && out.unreached.is_empty()
                    && out.undecided.is_empty()) =>
        {
            Err(
                "`CANNOT_START` goes with no `FINDING`, `UNREACHED` or `UNDECIDED` line"
                    .to_string(),
            )
        }
        Some(_) => Ok(out),
    }
}

/// What a run of the script is to be, with the paths of the copy it runs in.
struct Plan<'a> {
    request: &'a SimulateRequest,
    cfg: &'a SimConfig,
    script: &'a Path,
    interfaces: Vec<(&'a Path, &'a InterfaceText)>,
}

/// What a simulation has observed so far.
#[derive(Debug, Default)]
struct SimProgress {
    cases: Vec<CaseRun>,
    reruns: Vec<Rerun>,
    evidence: Vec<StepEvidence>,
}

impl ProcessRunner {
    /// A digest of the settings of a simulation of `harness` of `context`: the table as written and the
    /// interpreter's executable, so that a new interpreter is a new run.
    pub(super) async fn sim_settings_of(
        &self,
        context: &str,
        harness: &str,
    ) -> Result<String, RunnerError> {
        let cfg = self.sim_config(context)?;
        self.config.harness(context, harness).ok_or_else(|| {
            config_error(format!(
                "[[measure.layout]] has no entry for the harness `{harness}` of `{context}`"
            ))
        })?;
        let (program, _) = cfg
            .interpreter
            .split_first()
            .ok_or_else(|| config_error("the interpreter is empty".to_string()))?;
        let executable = process::locate(&self.root, program)
            .await
            .map_err(|f| config_error(format!("the interpreter: {}", f.message)))?;
        let interpreter = process::file_digest(&executable)
            .await
            .map_err(|f| config_error(format!("the interpreter: {}", f.message)))?;
        Ok(Digest::of(
            format!("config={};interpreter={interpreter}", self.config.digest).as_bytes(),
        )
        .to_string())
    }

    fn sim_config(&self, context: &str) -> Result<&SimConfig, RunnerError> {
        self.config.sim.get(context).ok_or_else(|| {
            config_error(format!(
                "[measure.sim.{context}] is not configured; it gives the interpreter and the program"
            ))
        })
    }

    /// Runs the simulation `request` asks for.
    pub(super) async fn simulate_run(
        &self,
        request: &SimulateRequest,
    ) -> Result<SimulateReport, RunnerError> {
        let cfg = self.sim_config(&request.context)?;
        let script = self
            .config
            .harness(&request.context, &request.harness)
            .ok_or_else(|| {
                config_error(format!(
                    "[[measure.layout]] has no entry for the harness `{}` of `{}`",
                    request.harness, request.context
                ))
            })?;
        let mut interfaces = Vec::with_capacity(request.interfaces.len());
        for i in &request.interfaces {
            let (place, _) = self
                .config
                .interface(&request.context, &i.name)
                .ok_or_else(|| {
                    config_error(format!(
                        "[[measure.layout]] has no entry for the interface `{}` of `{}`",
                        i.name, request.context
                    ))
                })?;
            interfaces.push((place.path.as_path(), i));
        }
        let dirs = self.run_dirs(&request.run)?;
        let deadline = process::deadline_in(cfg.budget).map_err(|f| config_error(f.message))?;
        let plan = Plan {
            request,
            cfg,
            script: script.path.as_path(),
            interfaces,
        };
        let first = Job {
            runner: self,
            dirs,
            deadline,
        };
        let mut progress = SimProgress::default();
        let finished = first.sim_steps(&plan, &mut progress).await;
        first.clean().await;
        Ok(SimulateReport {
            cases: progress.cases,
            reruns: progress.reruns,
            failure: finished.err(),
            evidence: progress.evidence,
        })
    }
}

impl Job<'_> {
    /// Copies the project, writes the pinned code and the script over the copy, and builds the program. `tag`
    /// names the steps in the evidence. The interpreter is returned, found in the copy.
    async fn sim_prepare(
        &self,
        plan: &Plan<'_>,
        tag: &str,
        out: &mut SimProgress,
    ) -> Result<Command, MeasureFailure> {
        let config = &self.runner.config;
        let (root, tree, exclude) = (
            self.runner.root.clone(),
            self.dirs.tree.clone(),
            config.copy_exclude.clone(),
        );
        tokio::task::spawn_blocking(move || super::copy_tree(&root, &tree, &exclude))
            .await
            .map_err(|e| io("the copy of the project", e))?
            .map_err(|e| io("cannot copy the project", e))?;
        for (path, i) in &plan.interfaces {
            self.write(path, &i.code).await?;
        }
        self.write(plan.script, &plan.request.script).await?;
        let env = self.env();
        let build = self.command(&config.build).await?;
        let started = Instant::now();
        let result = self.exec(&build, &env, config.baseline_timeout).await;
        let (exit_code, stderr) = match &result {
            Ok((ran, _)) => (ran.exit_code, provider::tail(&ran.stderr, TAIL)),
            Err(_) => (None, String::new()),
        };
        out.evidence.push(StepEvidence {
            step: format!("{tag}-build"),
            command: build.line(),
            command_digest: Some(build.digest.clone()),
            exit_code,
            duration_ms: millis(started),
            stderr,
        });
        let (ran, _) = result.map_err(|f| self.fail(f))?;
        if !ran.success {
            let said = [
                provider::tail(&ran.stderr, TAIL / 2),
                provider::tail(&ran.stdout, TAIL / 2),
            ]
            .join("\n");
            return Err(MeasureFailure::of(
                FailureKind::NoBuild,
                format!(
                    "the program does not build (exit {}): {}",
                    ran.exit_code
                        .map_or_else(|| "by a signal".to_string(), |c| c.to_string()),
                    said.trim()
                ),
            ));
        }
        self.command(&plan.cfg.interpreter).await
    }

    /// Runs the script for `case` and reads what it said and what it wrote.
    async fn sim_case(
        &self,
        plan: &Plan<'_>,
        interpreter: &Command,
        case: &SimCase,
        tag: &str,
        out: &mut SimProgress,
    ) -> Result<CaseRun, MeasureFailure> {
        if self.over_budget() {
            return Err(super::from_process(
                process::Failure {
                    kind: process::FailureKind::Timeout,
                    message: String::new(),
                },
                true,
            ));
        }
        let stem = case.to_string();
        let dir: PathBuf = self.dirs.dir.join(format!("out-{tag}")).join(&stem);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| io(&format!("cannot make {}", dir.display()), e))?;
        let mut args = interpreter.args.clone();
        args.push(
            self.dirs
                .tree
                .join(plan.script)
                .to_string_lossy()
                .into_owned(),
        );
        args.push(stem.clone());
        args.push("--out".to_string());
        args.push(dir.to_string_lossy().into_owned());
        args.push("--".to_string());
        args.extend(plan.cfg.program.iter().map(|w| self.dirs.expand(w)));
        let command = Command {
            executable: interpreter.executable.clone(),
            args,
            digest: interpreter.digest.clone(),
        };
        let env = self.env();
        let deadline = process::deadline_in(plan.cfg.timeout)
            .map_err(|f| self.fail(f))?
            .min(self.deadline);
        let started = Instant::now();
        let mut capture = Capture::default();
        let status = process::run(
            &process::Spec {
                executable: &command.executable,
                args: &command.args,
                cwd: &self.dirs.tree,
                env: &env,
                stdin: None,
                deadline,
                stdout: Pipe {
                    limit: REPORT_LIMIT,
                    over: Over::Fail,
                },
                stderr: Pipe {
                    limit: ERROR_LIMIT,
                    over: Over::Truncate,
                },
            },
            &mut capture,
        )
        .await;
        out.evidence.push(StepEvidence {
            step: format!("{tag}-{stem}"),
            command: command.line(),
            command_digest: Some(command.digest.clone()),
            exit_code: capture.exit_code,
            duration_ms: millis(started),
            stderr: provider::tail(&capture.stderr, TAIL),
        });
        let protocol =
            |why: String| MeasureFailure::of(FailureKind::Protocol, format!("{stem}: {why}"));
        // `Some` is an end that needs no report read; `None`, an exit 0 or 1 whose report is read.
        let ended = match status {
            Ok(_) => match capture.exit_code {
                Some(0 | 1) => None,
                Some(code) => Some(CaseExit::Script(code)),
                None => Some(CaseExit::Script(-1)),
            },
            Err(f) if f.kind == process::FailureKind::Timeout && !self.over_budget() => {
                Some(CaseExit::Timeout)
            }
            Err(f) if f.kind == process::FailureKind::Limit => {
                return Err(protocol("the report is longer than 1 MiB".to_string()));
            }
            Err(f) => return Err(self.fail(f)),
        };
        let reported = match ended {
            Some(_) => Reported::default(),
            None => parse_report(
                case,
                &plan.request.clauses,
                &String::from_utf8_lossy(&capture.stdout),
            )
            .map_err(protocol)?,
        };
        let exit = match (ended, &reported.cannot_start) {
            (Some(exit), _) => exit,
            // The script says it never started the program: a case that failed to run, with no findings, which
            // needs no schedule and trace since nothing was driven.
            (None, Some(why)) => CaseExit::CannotStart(why.clone()),
            // Exit 1 says there are findings; one that names none (and no unreached clause) is a script that
            // failed without saying why.
            (None, None)
                if capture.exit_code == Some(1)
                    && reported.findings.is_empty()
                    && reported.unreached.is_empty() =>
            {
                CaseExit::Script(1)
            }
            (None, None) => CaseExit::Normal,
        };
        let schedule = digest_of(&dir.join("schedule"))
            .await
            .map_err(|f| self.fail(f))?;
        let trace = digest_of(&dir.join("trace"))
            .await
            .map_err(|f| self.fail(f))?;
        if exit == CaseExit::Normal && (schedule.is_none() || trace.is_none()) {
            return Err(protocol(
                "ended normally without writing both `schedule` and `trace` into --out".to_string(),
            ));
        }
        Ok(CaseRun {
            case: case.clone(),
            exit,
            findings: reported.findings,
            unreached: reported.unreached,
            undecided: reported.undecided,
            schedule,
            trace,
        })
    }

    /// The whole simulation: build, every case, then the sampled cases again in a fresh copy.
    async fn sim_steps(
        &self,
        plan: &Plan<'_>,
        out: &mut SimProgress,
    ) -> Result<(), MeasureFailure> {
        let interpreter = self.sim_prepare(plan, "sim", out).await?;
        let mut cases: Vec<SimCase> = (0..plan.cfg.seeds).map(SimCase::Seed).collect();
        cases.extend(plan.request.regress.iter().cloned().map(SimCase::Regress));
        for case in &cases {
            let run = self.sim_case(plan, &interpreter, case, "sim", out).await?;
            out.cases.push(run);
        }
        // Every case said the program could not be started: nothing was checked, so the run is a failure of its
        // own kind and not a run with no findings. The second runs would repeat nothing.
        let unstarted: Vec<&CaseRun> = out.cases.iter().filter(|c| c.exit.cannot_start()).collect();
        if let Some(first) = unstarted
            .first()
            .filter(|_| unstarted.len() == out.cases.len())
        {
            let why = match &first.exit {
                CaseExit::CannotStart(why) => why.as_str(),
                _ => "",
            };
            return Err(MeasureFailure::of(
                FailureKind::NoStart,
                format!(
                    "the script could not start the program in any of the {} case(s); {}: {}",
                    unstarted.len(),
                    first.case,
                    why
                ),
            ));
        }
        let again = rerun_cases(&out.cases, plan.cfg.sample);
        if again.is_empty() {
            return Ok(());
        }
        // A second, fresh copy: what the first run left behind (files the program wrote, a cache) is not there.
        let second = Job {
            runner: self.runner,
            dirs: self.dirs.rerun(),
            deadline: self.deadline,
        };
        let interpreter = second.sim_prepare(plan, "rerun", out).await?;
        for case in &again {
            let run = second
                .sim_case(plan, &interpreter, case, "rerun", out)
                .await?;
            let Some(first) = out.cases.iter().find(|c| c.case == *case) else {
                continue;
            };
            let result = reproduction(first, &run);
            out.reruns.push(Rerun {
                case: case.clone(),
                result,
            });
        }
        Ok(())
    }
}

/// The digest of the file `path`; none when there is no such file.
async fn digest_of(path: &Path) -> Result<Option<Digest>, process::Failure> {
    if tokio::fs::metadata(path).await.is_err() {
        return Ok(None);
    }
    let hex = process::file_digest(path).await?;
    Ok(Digest::restore(&hex))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::clause::ClauseLabel;
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::sim::RegressName;

    /// The Clauses the sweep cites: stock:C1 (11), stock:C4 (14) and cli:C1 (21) (C1 is in both Contracts).
    fn cited() -> Vec<CitedClause> {
        [("stock", "C1", 11), ("stock", "C4", 14), ("cli", "C1", 21)]
            .iter()
            .map(|(contract, label, id)| CitedClause {
                name: ClauseName::new(contract, ClauseLabel::try_from(*label).unwrap()).unwrap(),
                id: clause(*id),
            })
            .collect()
    }

    fn clause(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    fn seed(n: u32) -> SimCase {
        SimCase::Seed(n)
    }

    #[test]
    fn a_report_of_every_form_is_read_line_by_line() {
        let text = "FINDING seed-3 step 7 stock:C4: capacity 0 accepted\n\
                    FINDING seed-3 step 9 stock:C1: a load: over the limit fits\n\
                    UNREACHED stock:C4: capacity below 1\n\
                    UNDECIDED more than 1000 outcomes\n\
                    \n\
                    sim: findings=2\n";
        let r = parse_report(&seed(3), &cited(), text).unwrap();
        assert_eq!(
            r.findings,
            [
                CaseFinding {
                    step: 7,
                    clause: clause(14),
                    text: "capacity 0 accepted".into()
                },
                CaseFinding {
                    step: 9,
                    clause: clause(11),
                    text: "a load: over the limit fits".into()
                },
            ]
        );
        assert_eq!(
            r.unreached,
            [CaseUnreached {
                clause: clause(14),
                text: "capacity below 1".into()
            }]
        );
        assert_eq!(r.undecided, ["more than 1000 outcomes"]);
        // a regress case names itself in full
        let regress = SimCase::Regress(RegressName::try_from("lease-lost").unwrap());
        let ok = parse_report(
            &regress,
            &cited(),
            "FINDING regress-lease-lost step 1 cli:C1: x\nsim: findings=1\n",
        )
        .unwrap();
        assert_eq!(ok.findings.len(), 1);
        assert_eq!(ok.findings[0].clause, clause(21));
        // a clean report is just the summary
        assert_eq!(
            parse_report(&seed(0), &cited(), "sim: findings=0\n").unwrap(),
            Reported::default()
        );
    }

    #[test]
    fn a_report_that_says_the_program_could_not_start_holds_that_and_nothing_else() {
        let ok = parse_report(
            &seed(3),
            &cited(),
            "CANNOT_START python3: No such file or directory\nsim: findings=0\n",
        )
        .unwrap();
        assert_eq!(
            ok.cannot_start.as_deref(),
            Some("python3: No such file or directory")
        );
        assert!(ok.findings.is_empty() && ok.unreached.is_empty());
        for text in [
            "CANNOT_START x\nFINDING seed-3 step 1 stock:C1: y\nsim: findings=1\n",
            "UNREACHED stock:C1: y\nCANNOT_START x\nsim: findings=0\n",
            "CANNOT_START x\nUNDECIDED y\nsim: findings=0\n",
        ] {
            let error = parse_report(&seed(3), &cited(), text).unwrap_err();
            assert!(
                error.contains("goes with no `FINDING`"),
                "{text:?}: {error}"
            );
        }
        let twice = parse_report(
            &seed(3),
            &cited(),
            "CANNOT_START x\nCANNOT_START y\nsim: findings=0\n",
        )
        .unwrap_err();
        assert!(twice.contains("twice"), "{twice}");
        // a start that gave no reason, or with no summary, is not a report
        for text in ["CANNOT_START \nsim: findings=0\n", "CANNOT_START x\n"] {
            assert!(parse_report(&seed(3), &cited(), text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_line_that_is_none_of_the_forms_is_refused_with_its_number() {
        for (text, expect) in [
            ("hello\nsim: findings=0\n", "line 1 `hello`"),
            ("sim-protocol: chidev-sim 2\nsim: findings=0\n", "line 1"),
            ("FINDING seed-3\nsim: findings=0\n", "no `<case> step"),
            (
                "FINDING seed-3 step 0 stock:C1: x\nsim: findings=1\n",
                "not a step",
            ),
            (
                "FINDING seed-3 step x stock:C1: x\nsim: findings=1\n",
                "not a step",
            ),
            (
                "FINDING seed-3 step 2 stock:C1 x\nsim: findings=1\n",
                "no `<contract>:<label>: <text>`",
            ),
            (
                "FINDING seed-3 step 2 stock:C1:\nsim: findings=1\n",
                "empty",
            ),
            (
                "FINDING seed-3 step 2 stock:c1: x\nsim: findings=1\n",
                "`stock:c1` is not a clause name",
            ),
            (
                "FINDING seed-4 step 2 stock:C1: x\nsim: findings=1\n",
                "`seed-4`, and this run is of `seed-3`",
            ),
            (
                "UNREACHED stock:C1\nsim: findings=0\n",
                "no `<contract>:<label>: <text>`",
            ),
            ("UNDECIDED\nsim: findings=0\n", "none of the forms"),
            ("UNDECIDED  \nsim: findings=0\n", "none of the forms"),
            ("sim: findings=x\n", "not a count"),
            ("sim: findings=-1\n", "not a count"),
        ] {
            let error = parse_report(&seed(3), &cited(), text).unwrap_err();
            assert!(error.contains(expect), "{text:?} gave {error}");
        }
    }

    #[test]
    fn a_clause_the_sweep_does_not_cite_is_refused() {
        for text in [
            "FINDING seed-3 step 2 stock:C9: x\nsim: findings=1\n",
            "UNREACHED stock:C9: x\nsim: findings=0\n",
            // C4 is stock's; cli's sweep citations are C1 only
            "UNREACHED cli:C4: x\nsim: findings=0\n",
        ] {
            let error = parse_report(&seed(3), &cited(), text).unwrap_err();
            assert!(error.contains("does not cite clause"), "{error}");
        }
    }

    #[test]
    fn a_contract_the_sweep_cites_nothing_of_is_refused_naming_the_ones_it_does() {
        for text in [
            "FINDING seed-3 step 2 fuel:C1: x\nsim: findings=1\n",
            "UNREACHED fuel:C1: x\nsim: findings=0\n",
        ] {
            let error = parse_report(&seed(3), &cited(), text).unwrap_err();
            assert!(
                error.contains("no contract `fuel`") && error.contains("cli, stock"),
                "{error}"
            );
        }
        // a name without a contract is no name at all
        let bare = parse_report(
            &seed(3),
            &cited(),
            "FINDING seed-3 step 2 C1: x\nsim: findings=1\n",
        )
        .unwrap_err();
        assert!(bare.contains("`C1` is not a clause name"), "{bare}");
    }

    #[test]
    fn one_label_in_two_contracts_is_two_clauses() {
        let r = parse_report(
            &seed(0),
            &cited(),
            "FINDING seed-0 step 1 stock:C1: a\nFINDING seed-0 step 2 cli:C1: b\nUNREACHED cli:C1: c\nsim: findings=2\n",
        )
        .unwrap();
        assert_eq!(
            r.findings.iter().map(|f| f.clause).collect::<Vec<_>>(),
            [clause(11), clause(21)]
        );
        assert_eq!(r.unreached[0].clause, clause(21));
    }

    #[test]
    fn the_summary_is_there_once_last_and_right() {
        let miss = parse_report(&seed(0), &cited(), "UNDECIDED x\n").unwrap_err();
        assert!(miss.contains("no `sim: findings=<n>`"), "{miss}");
        let empty = parse_report(&seed(0), &cited(), "\n\n").unwrap_err();
        assert!(empty.contains("empty"), "{empty}");
        let twice =
            parse_report(&seed(0), &cited(), "sim: findings=0\nsim: findings=0\n").unwrap_err();
        assert!(twice.contains("after the `sim:` summary"), "{twice}");
        let after = parse_report(&seed(0), &cited(), "sim: findings=0\nUNDECIDED x\n").unwrap_err();
        assert!(after.contains("line 2"), "{after}");
        let wrong = parse_report(
            &seed(0),
            &cited(),
            "FINDING seed-0 step 1 stock:C1: x\nsim: findings=2\n",
        )
        .unwrap_err();
        assert!(
            wrong.contains("findings=2") && wrong.contains("has 1"),
            "{wrong}"
        );
        let none = parse_report(&seed(0), &cited(), "sim: findings=1\n").unwrap_err();
        assert!(none.contains("has 0"), "{none}");
    }

    #[test]
    fn a_long_line_is_cut_in_the_message() {
        let line = "x".repeat(500);
        let error = parse_report(&seed(0), &cited(), &line).unwrap_err();
        assert!(error.len() < 300, "{}", error.len());
        assert!(error.contains("..."));
    }
}
