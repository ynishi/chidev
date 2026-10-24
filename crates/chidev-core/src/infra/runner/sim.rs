//! Running a sim Harness: the protocol `chidev-sim 1`, language-independent.
//!
//! chidev copies the project (as a mutation run does), writes the code the Pins of the cited Interfaces accepted and
//! the script over the copy, and runs the project's build command. It then starts the script once per case:
//!
//! ```text
//! <interpreter...> <script> <case> --out <dir> -- <program argv...>
//! ```
//!
//! `<case>` is `seed-<n>` or `regress-<name>`, the working directory is the copy, and `<program argv...>` is the
//! configured `program` with `{work}` made the root of the copy. The script drives the program as a black box
//! (standard input and output, arguments) and writes two files into `<dir>`: `schedule`, the choices it made for
//! this case from its seed, and `trace`, what the program did under them. chidev takes their SHA-256 digests; the
//! script does not report them. On standard output the script prints, one per line:
//!
//! ```text
//! FINDING <case> step <k> <ClauseLabel>: <text>   a promise of the Clause did not hold after step k (k from 1)
//! UNREACHED <ClauseLabel>: <text>                 this case never reached the situation the Clause needs
//! UNDECIDED <text>                                the script could not decide something (counted, never failed)
//! sim: findings=<n>                               last line; n is the number of FINDING lines
//! ```
//!
//! Exit 0 or 1 (1 when there are findings) means the report is to be read. A label the sweep Case does not cite, a
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
use super::config::{SimConfig, WORK_DIR};
use super::{Command, ERROR_LIMIT, Job, ProcessRunner, TAIL, config_error, io, millis, provider};
use crate::app::ports::measure::{InterfaceText, RunnerError, SimulateReport, SimulateRequest};
use crate::domain::vo::clause::ClauseLabel;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::measure::{FailureKind, MeasureFailure, StepEvidence};
use crate::domain::vo::sim::{
    CaseExit, CaseFinding, CaseRun, CaseUnreached, Rerun, SimCase, reproduction, rerun_cases,
};

/// The longest report a script may print.
const REPORT_LIMIT: usize = 1024 * 1024;

/// What a script said of one case.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Reported {
    pub findings: Vec<CaseFinding>,
    pub unreached: Vec<CaseUnreached>,
    pub undecided: Vec<String>,
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

/// `<Label>: <text>`, the label one that the sweep cites.
fn labelled<'a>(rest: &'a str, labels: &[ClauseLabel]) -> Result<(ClauseLabel, &'a str), String> {
    let (label, text) = rest
        .split_once(':')
        .ok_or_else(|| "no `<ClauseLabel>: <text>`".to_string())?;
    let parsed = ClauseLabel::try_from(label.trim())
        .map_err(|_| format!("`{}` is not a clause label (`C<n>`)", label.trim()))?;
    if !labels.contains(&parsed) {
        return Err(format!("the sweep case does not cite clause {parsed}"));
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("the text after the label is empty".to_string());
    }
    Ok((parsed, text))
}

/// `<case> step <k> <ClauseLabel>: <text>` of a `FINDING` line.
fn finding_of(rest: &str, case: &SimCase, labels: &[ClauseLabel]) -> Result<CaseFinding, String> {
    let (named, after) = rest
        .split_once(' ')
        .ok_or_else(|| "no `<case> step <k> <ClauseLabel>: <text>`".to_string())?;
    if named != case.to_string() {
        return Err(format!(
            "the finding is of `{named}`, and this run is of `{case}`"
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
    let (label, text) = labelled(after, labels)?;
    Ok(CaseFinding {
        step,
        label,
        text: text.to_string(),
    })
}

/// Reads the report of the script run for `case`. `labels` are the Clauses the sweep cites. Any line that is not
/// one of the forms, a label not cited, a finding of another case, no summary or two, a summary that is not the
/// last line, and a summary that counts other than the lines it follows are errors; the message says which line.
pub(super) fn parse_report(
    case: &SimCase,
    labels: &[ClauseLabel],
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
                .push(finding_of(rest, case, labels).map_err(bad)?);
        } else if let Some(rest) = line.strip_prefix("UNREACHED ") {
            let (label, text) = labelled(rest, labels).map_err(bad)?;
            out.unreached.push(CaseUnreached {
                label,
                text: text.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix("UNDECIDED ") {
            if rest.trim().is_empty() {
                return Err(bad("an empty reason".to_string()));
            }
            out.undecided.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("sim: findings=") {
            let count: u32 = rest
                .trim()
                .parse()
                .map_err(|_| bad(format!("`{}` is not a count", rest.trim())))?;
            summary = Some((at, count));
        } else {
            return Err(bad("none of the forms of chidev-sim 1".to_string()));
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
        let safe = !request.run.is_empty()
            && request
                .run
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !safe {
            return Err(config_error(format!(
                "`{}` is not a name for a run's directory",
                request.run
            )));
        }
        let deadline = process::deadline_in(cfg.budget).map_err(|f| config_error(f.message))?;
        let work = self.root.join(WORK_DIR).join(&request.run);
        let plan = Plan {
            request,
            cfg,
            script: script.path.as_path(),
            interfaces,
        };
        let first = Job {
            runner: self,
            tree: work.join("tree"),
            work: work.clone(),
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
            self.tree.clone(),
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
        let dir: PathBuf = self.work.join(format!("out-{tag}")).join(&stem);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| io(&format!("cannot make {}", dir.display()), e))?;
        let tree = self.tree.to_string_lossy().into_owned();
        let mut args = interpreter.args.clone();
        args.push(self.tree.join(plan.script).to_string_lossy().into_owned());
        args.push(stem.clone());
        args.push("--out".to_string());
        args.push(dir.to_string_lossy().into_owned());
        args.push("--".to_string());
        args.extend(plan.cfg.program.iter().map(|w| w.replace("{work}", &tree)));
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
                cwd: &self.tree,
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
                &plan.request.labels,
                &String::from_utf8_lossy(&capture.stdout),
            )
            .map_err(protocol)?,
        };
        let exit = match ended {
            Some(exit) => exit,
            // Exit 1 says there are findings; one that names none (and no unreached clause) is a script that
            // failed without saying why.
            None if capture.exit_code == Some(1)
                && reported.findings.is_empty()
                && reported.unreached.is_empty() =>
            {
                CaseExit::Script(1)
            }
            None => CaseExit::Normal,
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
        let again = rerun_cases(&out.cases, plan.cfg.sample);
        if again.is_empty() {
            return Ok(());
        }
        // A second, fresh copy: what the first run left behind (files the program wrote, a cache) is not there.
        let second = Job {
            runner: self.runner,
            work: self.work.clone(),
            tree: self.work.join("rerun-tree"),
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
    use crate::domain::vo::sim::RegressName;

    fn labels() -> Vec<ClauseLabel> {
        ["C1", "C4"]
            .iter()
            .map(|l| ClauseLabel::try_from(*l).unwrap())
            .collect()
    }

    fn label(n: &str) -> ClauseLabel {
        ClauseLabel::try_from(n).unwrap()
    }

    fn seed(n: u32) -> SimCase {
        SimCase::Seed(n)
    }

    #[test]
    fn a_report_of_every_form_is_read_line_by_line() {
        let text = "FINDING seed-3 step 7 C4: capacity 0 accepted\n\
                    FINDING seed-3 step 9 C1: a load: over the limit fits\n\
                    UNREACHED C4: capacity below 1\n\
                    UNDECIDED more than 1000 outcomes\n\
                    \n\
                    sim: findings=2\n";
        let r = parse_report(&seed(3), &labels(), text).unwrap();
        assert_eq!(
            r.findings,
            [
                CaseFinding {
                    step: 7,
                    label: label("C4"),
                    text: "capacity 0 accepted".into()
                },
                CaseFinding {
                    step: 9,
                    label: label("C1"),
                    text: "a load: over the limit fits".into()
                },
            ]
        );
        assert_eq!(
            r.unreached,
            [CaseUnreached {
                label: label("C4"),
                text: "capacity below 1".into()
            }]
        );
        assert_eq!(r.undecided, ["more than 1000 outcomes"]);
        // a regress case names itself in full
        let regress = SimCase::Regress(RegressName::try_from("lease-lost").unwrap());
        let ok = parse_report(
            &regress,
            &labels(),
            "FINDING regress-lease-lost step 1 C1: x\nsim: findings=1\n",
        )
        .unwrap();
        assert_eq!(ok.findings.len(), 1);
        // a clean report is just the summary
        assert_eq!(
            parse_report(&seed(0), &labels(), "sim: findings=0\n").unwrap(),
            Reported::default()
        );
    }

    #[test]
    fn a_line_that_is_none_of_the_forms_is_refused_with_its_number() {
        for (text, expect) in [
            ("hello\nsim: findings=0\n", "line 1 `hello`"),
            ("sim-protocol: chidev-sim 1\nsim: findings=0\n", "line 1"),
            ("FINDING seed-3\nsim: findings=0\n", "no `<case> step"),
            (
                "FINDING seed-3 step 0 C1: x\nsim: findings=1\n",
                "not a step",
            ),
            (
                "FINDING seed-3 step x C1: x\nsim: findings=1\n",
                "not a step",
            ),
            (
                "FINDING seed-3 step 2 C1 x\nsim: findings=1\n",
                "no `<ClauseLabel>: <text>`",
            ),
            ("FINDING seed-3 step 2 C1:\nsim: findings=1\n", "empty"),
            (
                "FINDING seed-3 step 2 c1: x\nsim: findings=1\n",
                "not a clause label",
            ),
            (
                "FINDING seed-4 step 2 C1: x\nsim: findings=1\n",
                "`seed-4`, and this run is of `seed-3`",
            ),
            (
                "UNREACHED C1\nsim: findings=0\n",
                "no `<ClauseLabel>: <text>`",
            ),
            ("UNDECIDED\nsim: findings=0\n", "none of the forms"),
            ("UNDECIDED  \nsim: findings=0\n", "none of the forms"),
            ("sim: findings=x\n", "not a count"),
            ("sim: findings=-1\n", "not a count"),
        ] {
            let error = parse_report(&seed(3), &labels(), text).unwrap_err();
            assert!(error.contains(expect), "{text:?} gave {error}");
        }
    }

    #[test]
    fn a_label_the_sweep_does_not_cite_is_refused() {
        for text in [
            "FINDING seed-3 step 2 C9: x\nsim: findings=1\n",
            "UNREACHED C9: x\nsim: findings=0\n",
        ] {
            let error = parse_report(&seed(3), &labels(), text).unwrap_err();
            assert!(error.contains("does not cite clause C9"), "{error}");
        }
    }

    #[test]
    fn the_summary_is_there_once_last_and_right() {
        let miss = parse_report(&seed(0), &labels(), "UNDECIDED x\n").unwrap_err();
        assert!(miss.contains("no `sim: findings=<n>`"), "{miss}");
        let empty = parse_report(&seed(0), &labels(), "\n\n").unwrap_err();
        assert!(empty.contains("empty"), "{empty}");
        let twice =
            parse_report(&seed(0), &labels(), "sim: findings=0\nsim: findings=0\n").unwrap_err();
        assert!(twice.contains("after the `sim:` summary"), "{twice}");
        let after =
            parse_report(&seed(0), &labels(), "sim: findings=0\nUNDECIDED x\n").unwrap_err();
        assert!(after.contains("line 2"), "{after}");
        let wrong = parse_report(
            &seed(0),
            &labels(),
            "FINDING seed-0 step 1 C1: x\nsim: findings=2\n",
        )
        .unwrap_err();
        assert!(
            wrong.contains("findings=2") && wrong.contains("has 1"),
            "{wrong}"
        );
        let none = parse_report(&seed(0), &labels(), "sim: findings=1\n").unwrap_err();
        assert!(none.contains("has 0"), "{none}");
    }

    #[test]
    fn a_long_line_is_cut_in_the_message() {
        let line = "x".repeat(500);
        let error = parse_report(&seed(0), &labels(), &line).unwrap_err();
        assert!(error.len() < 300, "{}", error.len());
        assert!(error.contains("..."));
    }
}
