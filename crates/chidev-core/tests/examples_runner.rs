//! The runner of examples against a shell script as the program: the examples are data, so the program only has to
//! answer by its arguments, and these use `sh`, which every unix has. Every run directory is under the project's
//! `.chidev/measure`, and the project is under the test target directory.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chidev_core::app::ports::measure::{
    ExamplesReport, ExamplesRequest, InterfaceText, Runner, RunnerError,
};
use chidev_core::domain::vo::example::{Example, ExampleName, Verdict, Why};
use chidev_core::domain::vo::measure::{FailureKind, SettingsDigest};
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::infra::project::{self, FILE};
use chidev_core::infra::runner::{self, ProcessRunner};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const WORK_DIR: &str = ".chidev/measure";

struct Project(PathBuf);

impl Project {
    fn new() -> Project {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "chidev-examples-runner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("prog.sh"), "program, working tree text\n").unwrap();
        std::fs::write(
            dir.join(FILE),
            r#"[measure]
build = ["sh", "-c", "grep -q PINNED prog.sh"]
env = { MARK = "marked" }

[measure.examples.ctx]
program = ["sh", "{tree}/prog.sh"]
timeout = 2
budget = 60

[[measure.layout]]
context = "ctx"
interface = "prog"
path = "prog.sh"

[[measure.layout]]
context = "ctx"
harness = "ex"
path = "ex"
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

    /// Writes the executable `name` of the project.
    fn script(&self, name: &str, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(self.0.join(name), text).unwrap();
        std::fs::set_permissions(self.0.join(name), std::fs::Permissions::from_mode(0o755))
            .unwrap();
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

/// The program as the Pin accepted it: it answers by its first argument.
const PROGRAM: &str = r#"# PINNED
case "$1" in
  echo) cat ;;
  exit3) cat > /dev/null; exit 3 ;;
  sleep) sleep 30 ;;
  mark) printf '%s\n' "$MARK" ;;
  where) pwd ;;
esac
"#;

fn context(name: &str) -> ContextName {
    ContextName::try_from(name).unwrap()
}

fn asset(name: &str) -> AssetName {
    AssetName::try_from(name).unwrap()
}

/// The example `name`, started with `args`, given `stdin`, expected to give `expected` (ending with the exit line).
fn example(name: &str, args: &str, stdin: &str, expected: &str) -> (ExampleName, Example) {
    (
        ExampleName::new(name).unwrap(),
        Example::new(
            args.to_string(),
            stdin.to_string(),
            expected.to_string(),
            None,
        )
        .unwrap(),
    )
}

fn request(run: &str, examples: Vec<(ExampleName, Example)>) -> ExamplesRequest {
    ExamplesRequest {
        run: run.to_string(),
        context: context("ctx"),
        harness: asset("ex"),
        interfaces: vec![InterfaceText {
            name: asset("prog"),
            code: PROGRAM.to_string(),
        }],
        examples,
    }
}

fn verdicts(report: &ExamplesReport) -> Vec<(String, String)> {
    report
        .examples
        .iter()
        .map(|(name, verdict)| (name.to_string(), verdict.to_string()))
        .collect()
}

fn steps(report: &ExamplesReport) -> Vec<&str> {
    report.evidence.iter().map(|e| e.step.as_str()).collect()
}

#[tokio::test]
async fn each_example_is_run_against_the_pinned_program_and_judged_as_accept_judges_it() {
    let project = Project::new();
    let report = project
        .runner()
        .run_examples(&request(
            "run-1",
            vec![
                example("a1", "echo", "hi\n", "hi\n#exit 0\n"),
                example("a2", "exit3", "", "#exit 0\n"),
                example("a3", "echo", "x\n", "y\n#exit 0\n"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    assert_eq!(
        verdicts(&report),
        [
            ("a1".to_string(), "ok".to_string()),
            ("a2".to_string(), "exit code: expected 0, got 3".to_string()),
            (
                "a3".to_string(),
                "standard output differs at line 1: expected \"y\", got \"x\"".to_string()
            ),
        ]
    );
    // the program run is the pinned one: the build looked for it in the copy, and the evidence says so
    assert_eq!(steps(&report), ["examples-build"]);
    assert_eq!(report.evidence[0].exit_code, Some(0));
    // the project is untouched and the run's directory is gone
    assert_eq!(
        std::fs::read_to_string(project.0.join("prog.sh")).unwrap(),
        "program, working tree text\n"
    );
    assert!(
        std::fs::read_dir(project.0.join(WORK_DIR))
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn the_program_runs_in_the_copy_with_the_environment_of_the_table() {
    let project = Project::new();
    let report = project
        .runner()
        .run_examples(&request(
            "run-env",
            vec![
                example("env", "mark", "", "marked\n#exit 0\n"),
                example("cwd", "where", "", "elsewhere\n#exit 0\n"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    let all = verdicts(&report);
    // the variable of `env` reached the example's process
    assert_eq!(all[0].1, "ok", "{all:?}");
    // the working directory is the copy, under the project's work directory, and not the project
    let at = &all[1].1;
    assert!(at.contains(WORK_DIR) && at.contains("/tree"), "{at}");
}

#[tokio::test]
async fn an_example_that_does_not_end_within_the_timeout_fails_with_one_reason_and_the_run_goes_on()
{
    let project = Project::new();
    project.edit(|t| t.replace("timeout = 2", "timeout = 1"));
    let report = project
        .runner()
        .run_examples(&request(
            "run-slow",
            vec![
                example("slow", "sleep", "", "#exit 0\n"),
                example("after", "echo", "again\n", "again\n#exit 0\n"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    let Verdict::Fail(reasons) = &report.examples[0].1 else {
        panic!("the slow example passed");
    };
    assert_eq!(
        reasons,
        &[Why::Abnormal("did not end within 1 s".to_string())]
    );
    assert_eq!(report.examples[1].1, Verdict::Ok);
}

#[tokio::test]
async fn the_budget_ends_the_run_and_keeps_the_examples_it_got_through() {
    let project = Project::new();
    project.edit(|t| {
        t.replace("timeout = 2", "timeout = 30")
            .replace("budget = 60", "budget = 2")
    });
    let report = project
        .runner()
        .run_examples(&request(
            "run-budget",
            vec![
                example("quick", "echo", "ok\n", "ok\n#exit 0\n"),
                example("slow", "sleep", "", "#exit 0\n"),
                example("never", "echo", "", "#exit 0\n"),
            ],
        ))
        .await
        .unwrap();
    let failure = report.failure.clone().expect("a failure");
    assert_eq!(failure.kind(), FailureKind::Budget, "{failure}");
    // the example the budget cut short says nothing of the program and is not kept
    assert_eq!(verdicts(&report), [("quick".to_string(), "ok".to_string())]);
}

#[tokio::test]
async fn a_program_that_does_not_build_is_a_failed_run_with_no_example_run() {
    let project = Project::new();
    let mut bad = request("run-nobuild", vec![example("a1", "echo", "", "#exit 0\n")]);
    bad.interfaces[0].code = "no marker here\n".to_string();
    let report = project.runner().run_examples(&bad).await.unwrap();
    let failure = report.failure.clone().expect("a failure");
    assert_eq!(failure.kind(), FailureKind::NoBuild, "{failure}");
    assert!(report.examples.is_empty());
    assert_eq!(steps(&report), ["examples-build"]);
}

#[tokio::test]
async fn a_configuration_that_cannot_run_the_examples_is_an_error_and_runs_nothing() {
    let project = Project::new();
    let runner = project.runner();
    let ask = |f: &dyn Fn(&mut ExamplesRequest)| {
        let mut r = request("run-x", vec![example("a1", "echo", "", "#exit 0\n")]);
        f(&mut r);
        r
    };
    // no table for the Context
    let error = runner
        .run_examples(&ask(&|r| r.context = context("other")))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        RunnerError::NoExamplesConfig {
            context: context("other")
        }
    );
    assert!(error.to_string().contains("[measure.examples.other]"));
    // the Harness is not laid out
    let error = runner
        .run_examples(&ask(&|r| r.harness = asset("elsewhere")))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no entry for the harness `elsewhere`"),
        "{error}"
    );
    // an Interface that is not laid out
    let error = runner
        .run_examples(&ask(&|r| r.interfaces[0].name = asset("gone")))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no entry for the interface `gone`"),
        "{error}"
    );
    // a name that could lead out of the directory the runs work in
    let error = runner
        .run_examples(&ask(&|r| r.run = "../escape".to_string()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not a name"), "{error}");
    assert!(!project.0.join(WORK_DIR).exists());
}

#[tokio::test]
async fn a_program_that_is_not_there_is_an_error_naming_the_program() {
    let project = Project::new();
    project.edit(|t| t.replace("program = [\"sh\",", "program = [\"no-such-program-here\","));
    let error = project
        .runner()
        .run_examples(&request(
            "run-missing",
            vec![example("a1", "echo", "", "#exit 0\n")],
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::Locate { .. }), "{error:?}");
    assert!(error.to_string().starts_with("the program: "), "{error}");
    // and its settings cannot be read either
    let error = project
        .runner()
        .examples_settings(&context("ctx"), &asset("ex"), &[asset("prog")])
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::Locate { .. }), "{error:?}");
    assert!(!project.0.join(WORK_DIR).exists());
}

/// The digest of the settings of a run of the examples of `ex`, which builds `prog`.
async fn digest(project: &Project) -> SettingsDigest {
    project
        .runner()
        .examples_settings(&context("ctx"), &asset("ex"), &[asset("prog")])
        .await
        .unwrap()
}

#[tokio::test]
async fn the_settings_are_for_a_harness_and_the_interfaces_it_builds() {
    let project = Project::new();
    let (ctx, ex, prog) = (context("ctx"), asset("ex"), [asset("prog")]);
    let one = project
        .runner()
        .examples_settings(&ctx, &ex, &prog)
        .await
        .unwrap();
    assert_eq!(one, digest(&project).await);
    // another Harness, another Context, or an Interface that is not laid out has no settings
    assert_eq!(
        project
            .runner()
            .examples_settings(&context("other"), &ex, &prog)
            .await
            .unwrap_err(),
        RunnerError::NoExamplesConfig {
            context: context("other")
        }
    );
    for error in [
        project
            .runner()
            .examples_settings(&ctx, &asset("other"), &prog)
            .await
            .unwrap_err(),
        project
            .runner()
            .examples_settings(&ctx, &ex, &[asset("gone")])
            .await
            .unwrap_err(),
    ] {
        assert!(!error.to_string().is_empty());
    }
    // what the Harness builds is read by the run, so it is in the digest
    let none = project
        .runner()
        .examples_settings(&ctx, &ex, &[])
        .await
        .unwrap();
    assert_ne!(one, none);
}

#[tokio::test]
async fn a_setting_a_run_of_examples_does_not_read_leaves_its_digest_as_it_was() {
    let project = Project::new();
    let before = digest(&project).await;
    let text = std::fs::read_to_string(project.0.join(FILE)).unwrap();
    // the layout entries in the other order
    let mut parts: Vec<&str> = text.split("[[measure.layout]]").collect();
    parts.swap(1, 2);
    let swapped = parts.join("[[measure.layout]]");
    assert_ne!(swapped, text);
    let top = |keys: &str| text.replacen("[measure]\n", &format!("[measure]\n{keys}\n"), 1);
    for (what, table) in [
        ("test", top("test = [\"true\"]")),
        (
            "the limits of a mutation run",
            top("timeout = 9\nbudget = 99\nmax_mutants = 2\nseed = 5\noperators = [\"rel\"]"),
        ),
        (
            "the providers of mutants and of items",
            format!(
                "{text}\n[measure.mutants]\ntext = [\"sh\"]\n[measure.items]\ntext = [\"sh\"]\n"
            ),
        ),
        (
            "the sim of any context",
            format!(
                "{text}\n[measure.sim.ctx]\ninterpreter = [\"sh\"]\nprogram = [\"x\"]\nseeds = 4\n"
            ),
        ),
        (
            "the examples of another context",
            format!("{text}\n[measure.examples.other]\nprogram = [\"sh\"]\ntimeout = 3\n"),
        ),
        (
            "an interface or a harness it does not build or run",
            format!(
                "{text}\n[[measure.layout]]\ncontext = \"ctx\"\ninterface = \"else\"\npath = \"else.txt\"\n[[measure.layout]]\ncontext = \"ctx\"\nharness = \"unrun\"\npath = \"unrun.txt\"\n"
            ),
        ),
        ("the layout in another order", swapped),
    ] {
        project.edit(|_| table.clone());
        assert_eq!(digest(&project).await, before, "{what}");
    }
}

#[tokio::test]
async fn each_setting_a_run_of_examples_reads_moves_its_digest() {
    let project = Project::new();
    let before = digest(&project).await;
    let text = std::fs::read_to_string(project.0.join(FILE)).unwrap();
    let top = |keys: &str| text.replacen("[measure]\n", &format!("[measure]\n{keys}\n"), 1);
    let mut seen = vec![before];
    for (what, table) in [
        ("copy_exclude", top("copy_exclude = [\".git\"]")),
        ("env", text.replace("MARK = \"marked\"", "MARK = \"other\"")),
        (
            "build",
            text.replace("grep -q PINNED prog.sh", "grep -q PINNED2 prog.sh"),
        ),
        ("baseline_timeout", top("baseline_timeout = 7")),
        (
            "the program's command",
            text.replace("program = [\"sh\",", "program = [\"sh\", \"-e\","),
        ),
        ("timeout", text.replace("timeout = 2", "timeout = 3")),
        ("budget", text.replace("budget = 60", "budget = 61")),
        (
            "the path of the harness",
            text.replace("path = \"ex\"", "path = \"ex2\""),
        ),
        (
            "the path of a built interface",
            text.replace("path = \"prog.sh\"", "path = \"prog2.sh\""),
        ),
    ] {
        assert_ne!(table, text, "{what} is in the table");
        project.edit(|_| table.clone());
        let now = digest(&project).await;
        assert!(!seen.contains(&now), "{what} moves the digest");
        seen.push(now);
    }
}

#[tokio::test]
async fn a_table_with_its_defaults_written_out_and_one_without_have_one_digest() {
    let project = Project::new();
    let bare = |t: &str| {
        t.replace("env = { MARK = \"marked\" }\n", "")
            .replace("timeout = 2\n", "")
            .replace("budget = 60\n", "")
    };
    project.edit(bare);
    let before = digest(&project).await;
    project.edit(|t| {
        t.replacen(
            "[measure]\n",
            "[measure]\ncopy_exclude = [\"target\", \".chidev\", \".git\"]\nbaseline_timeout = 600\nenv = { CARGO_TARGET_DIR = \"{run}/target\" }\n",
            1,
        )
        .replacen(
            "program = [\"sh\", \"{tree}/prog.sh\"]\n",
            "program = [\"sh\", \"{tree}/prog.sh\"]\ntimeout = 10\nbudget = 600\n",
            1,
        )
    });
    assert_eq!(digest(&project).await, before);
}

#[tokio::test]
async fn the_programs_file_is_in_the_digest_and_the_report_carries_the_digest_the_run_ran_under() {
    let project = Project::new();
    project.script("launch.sh", "#!/bin/sh\nexec sh \"$@\"\n");
    project.edit(|t| t.replace("program = [\"sh\",", "program = [\"./launch.sh\","));
    let before = digest(&project).await;
    project.script("launch.sh", "#!/bin/sh\nexec sh \"$@\" # changed\n");
    let after = digest(&project).await;
    assert_ne!(before, after);
    let report = project
        .runner()
        .run_examples(&request(
            "run-digest",
            vec![example("a1", "echo", "hi\n", "hi\n#exit 0\n")],
        ))
        .await
        .unwrap();
    assert_eq!(report.failure, None, "{report:#?}");
    assert_eq!(report.settings, after);
    assert_eq!(report.examples[0].1, Verdict::Ok);
}

#[tokio::test]
async fn a_program_that_is_another_file_in_the_copy_than_at_the_root_fails_the_run_as_settings_moved()
 {
    let project = Project::new();
    project.script("launch.sh", "#!/bin/sh\nexec sh \"$@\"\n");
    // the pinned code is written over `launch.sh` in the copy
    project.edit(|t| {
        t.replace("program = [\"sh\",", "program = [\"./launch.sh\",")
            .replace("path = \"prog.sh\"", "path = \"launch.sh\"")
    });
    let report = project
        .runner()
        .run_examples(&request(
            "run-moved",
            vec![example("a1", "echo", "hi\n", "hi\n#exit 0\n")],
        ))
        .await
        .unwrap();
    let failure = report.failure.expect("a failure");
    assert_eq!(failure.kind(), FailureKind::SettingsMoved, "{failure}");
    assert!(failure.message().contains("`./launch.sh`"), "{failure}");
    assert!(report.examples.is_empty());
    assert!(!project.0.join(WORK_DIR).join("run-moved").exists());
}
