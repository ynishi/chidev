//! The runner of simulations against a shell script as "interpreter" and as "program": the protocol `chidev-sim 2`
//! is the same for any language, so these use `sh`, which every unix has. Every run directory is under the test
//! target directory, and the runner's own are under the project's `.chidev/measure`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use chidev_core::app::ports::measure::{InterfaceText, Runner, SimulateReport, SimulateRequest};
use chidev_core::domain::vo::clause::ClauseLabel;
use chidev_core::domain::vo::digest::Digest;
use chidev_core::domain::vo::id::ClauseId;
use chidev_core::domain::vo::measure::FailureKind;
use chidev_core::domain::vo::sim::{CitedClause, ClauseName, RegressName};
use chidev_core::infra::project::{self, FILE};
use chidev_core::infra::runner::{self, ProcessRunner};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const WORK_DIR: &str = ".chidev/measure";

struct Project(PathBuf);

impl Project {
    fn new(script: &str) -> Project {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "chidev-sim-runner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("prog.sh"), "program, working tree text\n").unwrap();
        std::fs::write(dir.join("script.sh"), script).unwrap();
        std::fs::write(
            dir.join(FILE),
            r#"[measure]
build = ["sh", "-c", "grep -q PINNED prog.sh"]
env = {}

[measure.sim.ctx]
interpreter = ["sh"]
program = ["{tree}/prog.sh"]
seeds = 3
sample = 2
timeout = 5
budget = 60

[[measure.layout]]
context = "ctx"
interface = "prog"
path = "prog.sh"

[[measure.layout]]
context = "ctx"
harness = "sim"
path = "script.sh"
"#,
        )
        .unwrap();
        Project(dir)
    }

    /// Changes `chidev.toml`.
    fn edit(&self, f: impl Fn(&str) -> String) {
        let path = self.0.join(FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, f(&text)).unwrap();
    }

    fn runner(&self) -> ProcessRunner {
        runner::load(&project::find(&self.0).unwrap())
            .unwrap()
            .expect("a [measure] table")
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The arguments of the script are `<case> --out <dir> -- <program...>`: `$1` is the case and `$3` the directory.
/// It writes a schedule and a trace from the case, and finds `stock:C1` in `seed-1`.
const GOOD: &str = r#"
case=$1
out=$3
printf 'schedule of %s\n' "$case" > "$out/schedule"
printf 'trace of %s\n' "$case" > "$out/trace"
if [ "$case" = seed-1 ]; then
  echo "FINDING seed-1 step 4 stock:C1: promise broken"
  echo "sim: findings=1"
  exit 1
fi
echo "sim: findings=0"
"#;

fn request(run: &str, script: &str) -> SimulateRequest {
    SimulateRequest {
        run: run.to_string(),
        context: "ctx".to_string(),
        harness: "sim".to_string(),
        script: script.to_string(),
        interfaces: vec![InterfaceText {
            name: "prog".to_string(),
            code: "PINNED program text\n".to_string(),
        }],
        // C1 is a Clause of both Contracts the sweep cites: `stock` (id 1) and `cli` (id 2)
        clauses: vec![cited("stock", "C1", 1), cited("cli", "C1", 2)],
        regress: vec![RegressName::try_from("edge").unwrap()],
    }
}

fn cited(contract: &str, label: &str, id: u128) -> CitedClause {
    CitedClause {
        name: ClauseName::new(contract, ClauseLabel::try_from(label).unwrap()).unwrap(),
        id: ClauseId::from_uuid(uuid::Uuid::from_u128(id)),
    }
}

fn cases(report: &SimulateReport) -> Vec<(String, String, usize)> {
    report
        .cases
        .iter()
        .map(|c| (c.case.to_string(), c.exit.word(), c.findings.len()))
        .collect()
}

fn steps(report: &SimulateReport) -> Vec<&str> {
    report.evidence.iter().map(|e| e.step.as_str()).collect()
}

fn row(case: &str, exit: &str, findings: usize) -> (String, String, usize) {
    (case.to_string(), exit.to_string(), findings)
}

#[tokio::test]
async fn every_case_is_run_the_findings_are_read_and_the_sampled_cases_are_repeated() {
    let project = Project::new(GOOD);
    let report = project
        .runner()
        .simulate(&request("run-1", GOOD))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    assert_eq!(
        cases(&report),
        [
            row("seed-0", "normal", 0),
            row("seed-1", "normal", 1),
            row("seed-2", "normal", 0),
            row("regress-edge", "normal", 0),
        ]
    );
    assert_eq!(report.cases[1].findings[0].step, 4);
    assert_eq!(report.cases[1].findings[0].text, "promise broken");
    // the name `stock:C1` was resolved to the Clause of `stock`, not to the `C1` of `cli`
    assert_eq!(
        report.cases[1].findings[0].clause,
        ClauseId::from_uuid(uuid::Uuid::from_u128(1))
    );
    // chidev took the digest of what the script wrote
    assert_eq!(
        report.cases[0].schedule,
        Some(Digest::of(b"schedule of seed-0\n"))
    );
    assert_eq!(
        report.cases[0].trace,
        Some(Digest::of(b"trace of seed-0\n"))
    );
    // seed-0 and the case with a finding were run again, and are the same
    let again: Vec<_> = report
        .reruns
        .iter()
        .map(|r| (r.case.to_string(), r.result.word()))
        .collect();
    assert_eq!(
        again,
        [
            ("seed-0".to_string(), "same"),
            ("seed-1".to_string(), "same")
        ]
    );
    // the evidence: the build and every case, then the second copy's build and the repeated cases
    assert_eq!(
        steps(&report),
        [
            "sim-build",
            "sim-seed-0",
            "sim-seed-1",
            "sim-seed-2",
            "sim-regress-edge",
            "rerun-build",
            "rerun-seed-0",
            "rerun-seed-1"
        ]
    );
    assert!(report.evidence.iter().all(|e| e.command_digest.is_some()));
    // the project's own files are untouched and the run's directory is gone
    assert_eq!(
        std::fs::read_to_string(project.0.join("prog.sh")).unwrap(),
        "program, working tree text\n"
    );
    assert!(!project.0.join(WORK_DIR).join("run-1").exists());
}

#[tokio::test]
async fn the_script_gets_the_documented_arguments() {
    // the script writes its own arguments as the schedule and the trace
    let script = r#"
printf '%s\n' "$@" > "$3/schedule"
cp "$3/schedule" "$3/trace"
echo "sim: findings=0"
"#;
    let project = Project::new(script);
    let report = project
        .runner()
        .simulate(&request("run-args", script))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    let root = project.0.canonicalize().unwrap();
    let run = root.join(WORK_DIR).join("run-args");
    let expected = format!(
        "seed-0\n--out\n{}\n--\n{}\n",
        run.join("out-sim").join("seed-0").display(),
        run.join("tree").join("prog.sh").display()
    );
    assert_eq!(
        report.cases[0].schedule,
        Some(Digest::of(expected.as_bytes())),
        "expected the arguments\n{expected}"
    );
}

#[tokio::test]
async fn a_program_that_does_not_build_fails_the_run_before_any_case() {
    let project = Project::new(GOOD);
    let mut r = request("run-2", GOOD);
    r.interfaces[0].code = "no marker\n".to_string();
    let report = project.runner().simulate(&r).await.unwrap();
    let failure = report.failure.clone().expect("a failure");
    assert_eq!(failure.kind(), FailureKind::NoBuild, "{failure}");
    assert!(report.cases.is_empty());
    assert_eq!(steps(&report), ["sim-build"]);
    assert_eq!(report.evidence[0].exit_code, Some(1));
}

/// A script that finds the program missing in the cases `missing` matches (a shell pattern for `$case`) and runs
/// as `GOOD` in the others.
fn cannot_start_in(missing: &str) -> String {
    format!(
        r#"
case=$1
out=$3
case "$case" in
  {missing})
    printf 'schedule of %s\n' "$case" > "$out/schedule"
    echo "CANNOT_START $5: not found"
    echo "sim: findings=0"
    exit 0 ;;
esac
{GOOD}
"#
    )
}

#[tokio::test]
async fn cases_that_could_not_start_the_program_are_counted_apart_and_none_started_fails_the_run() {
    // seed-0 and the regress case could not start the program; seed-1 and seed-2 drove it
    let script = cannot_start_in("seed-0|regress-edge");
    let project = Project::new(&script);
    let report = project
        .runner()
        .simulate(&request("run-9", &script))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    let exits: Vec<(String, bool, usize)> = report
        .cases
        .iter()
        .map(|c| (c.case.to_string(), c.exit.cannot_start(), c.findings.len()))
        .collect();
    assert_eq!(
        exits,
        [
            ("seed-0".to_string(), true, 0),
            ("seed-1".to_string(), false, 1),
            ("seed-2".to_string(), false, 0),
            ("regress-edge".to_string(), true, 0),
        ]
    );
    assert!(report.cases[0].exit.word().starts_with("cannot_start:"));
    // the case that could not start was not repeated
    assert!(
        report.reruns.iter().all(|r| r.case.to_string() != "seed-0"),
        "{:?}",
        report.reruns
    );

    // no case started the program: the run fails with its own kind, and the second runs are not made
    let script = cannot_start_in("*");
    let project = Project::new(&script);
    let report = project
        .runner()
        .simulate(&request("run-10", &script))
        .await
        .unwrap();
    let failure = report.failure.clone().expect("a failure");
    assert_eq!(failure.kind(), FailureKind::NoStart, "{failure}");
    assert!(
        failure.message().contains("any of the 4 case(s)"),
        "{failure}"
    );
    assert_eq!(report.cases.len(), 4);
    assert!(report.cases.iter().all(|c| c.exit.cannot_start()));
    assert!(report.cases.iter().all(|c| c.findings.is_empty()));
    assert!(report.reruns.is_empty());
    assert!(!steps(&report).iter().any(|s| s.starts_with("rerun")));
    assert!(!project.0.join(WORK_DIR).join("run-10").exists());
}

#[tokio::test]
async fn a_report_that_breaks_the_protocol_fails_the_run() {
    for (script, expect) in [
        (
            GOOD.replace("seed-1 step 4 stock:C1", "seed-1 step 4 stock:C7"),
            "does not cite clause stock:C7",
        ),
        (
            GOOD.replace("seed-1 step 4 stock:C1", "seed-1 step 4 fuel:C1"),
            "no contract `fuel` among those the sweep cites (cli, stock)",
        ),
        (
            // a name with no contract is the old protocol's: refused, not guessed at
            GOOD.replace("seed-1 step 4 stock:C1", "seed-1 step 4 C1"),
            "`C1` is not a clause name",
        ),
        (
            GOOD.replace("echo \"sim: findings=0\"", "echo hi"),
            "none of the forms",
        ),
        (
            GOOD.replace("echo \"sim: findings=0\"", ":"),
            "findings=<n>",
        ),
        (
            GOOD.replace("printf 'trace of %s\\n' \"$case\" > \"$out/trace\"", ":"),
            "without writing both",
        ),
    ] {
        let project = Project::new(&script);
        let report = project
            .runner()
            .simulate(&request("run-3", &script))
            .await
            .unwrap();
        let failure = report.failure.expect("a failure");
        assert_eq!(failure.kind(), FailureKind::Protocol, "{failure}");
        assert!(failure.message().contains(expect), "{failure} vs {expect}");
        assert!(!project.0.join(WORK_DIR).join("run-3").exists());
    }
}

#[tokio::test]
async fn a_script_that_fails_or_is_too_slow_for_a_case_is_recorded_and_the_run_goes_on() {
    let script = GOOD.replace(
        "echo \"sim: findings=0\"",
        "if [ \"$case\" = seed-0 ]; then exit 3; fi\nif [ \"$case\" = seed-2 ]; then sleep 30; fi\necho \"sim: findings=0\"",
    );
    let project = Project::new(&script);
    project.edit(|t| t.replace("timeout = 5", "timeout = 1"));
    let started = Instant::now();
    let report = project
        .runner()
        .simulate(&request("run-4", &script))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    assert_eq!(
        cases(&report),
        [
            row("seed-0", "script:3", 0),
            row("seed-1", "normal", 1),
            row("seed-2", "timeout", 0),
            row("regress-edge", "normal", 0),
        ]
    );
    assert!(started.elapsed().as_secs() < 25);
}

/// A script whose `which` file (`schedule` or `trace`) comes from a counter outside the copy that only grows, so
/// no two runs of it agree.
fn counting(which: &str) -> (String, PathBuf) {
    let counter = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "chidev-sim-counter-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let from = format!("printf '{which} of %s\\n' \"$case\" > \"$out/{which}\"");
    assert!(GOOD.contains(&from), "{from}");
    let script = GOOD.replace(
        &from,
        &format!(
            "echo x >> '{c}'; wc -l < '{c}' > \"$out/{which}\"",
            c = counter.display()
        ),
    );
    (script, counter)
}

#[tokio::test]
async fn a_script_that_is_not_repeatable_is_the_scripts_fault_and_a_program_that_is_not_is_the_programs()
 {
    for (which, expect) in [("schedule", "schedule-differs"), ("trace", "trace-differs")] {
        let (script, counter) = counting(which);
        let project = Project::new(&script);
        let report = project
            .runner()
            .simulate(&request("run-5", &script))
            .await
            .unwrap();
        let _ = std::fs::remove_file(&counter);
        assert_eq!(report.failure, None, "{report:#?}");
        assert_eq!(report.reruns.len(), 2);
        assert!(
            report.reruns.iter().all(|r| r.result.word() == expect),
            "{which}: {:?}",
            report.reruns
        );
    }
}

#[tokio::test]
async fn the_budget_ends_the_run_and_keeps_the_cases_it_got_through() {
    let script = GOOD.replace(
        "echo \"sim: findings=0\"",
        "if [ \"$case\" = seed-2 ]; then sleep 30; fi\necho \"sim: findings=0\"",
    );
    let project = Project::new(&script);
    // a budget of 2 seconds and 5 for a case: the sleeping case runs into the budget
    project.edit(|t| t.replace("budget = 60", "budget = 2"));
    let report = project
        .runner()
        .simulate(&request("run-7", &script))
        .await
        .unwrap();
    let failure = report.failure.expect("a failure");
    assert_eq!(failure.kind(), FailureKind::Budget, "{failure}");
    assert_eq!(report.cases.len(), 2);
}

#[tokio::test]
async fn a_configuration_that_cannot_simulate_is_an_error_and_runs_nothing() {
    let project = Project::new(GOOD);
    let runner = project.runner();
    let mut other_context = request("run-8", GOOD);
    other_context.context = "other".to_string();
    assert!(
        runner
            .simulate(&other_context)
            .await
            .unwrap_err()
            .to_string()
            .contains("[measure.sim.other]")
    );
    let mut no_harness = request("run-9", GOOD);
    no_harness.harness = "elsewhere".to_string();
    assert!(
        runner
            .simulate(&no_harness)
            .await
            .unwrap_err()
            .to_string()
            .contains("no entry for the harness `elsewhere`")
    );
    let mut no_interface = request("run-10", GOOD);
    no_interface.interfaces[0].name = "gone".to_string();
    assert!(
        runner
            .simulate(&no_interface)
            .await
            .unwrap_err()
            .to_string()
            .contains("no entry for the interface `gone`")
    );
    assert!(
        runner
            .simulate(&request("../escape", GOOD))
            .await
            .unwrap_err()
            .to_string()
            .contains("not a name")
    );
    assert!(!project.0.join(WORK_DIR).exists());
}

#[tokio::test]
async fn the_settings_follow_the_table_and_the_interpreter() {
    let project = Project::new(GOOD);
    let one = project.runner().sim_settings("ctx", "sim").await.unwrap();
    assert_eq!(
        one,
        project.runner().sim_settings("ctx", "sim").await.unwrap()
    );
    project.edit(|t| t.replace("seeds = 3", "seeds = 4"));
    let two = project.runner().sim_settings("ctx", "sim").await.unwrap();
    assert_ne!(one, two);
    assert!(project.runner().sim_settings("ctx", "other").await.is_err());
    assert!(project.runner().sim_settings("other", "sim").await.is_err());
}
