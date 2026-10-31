//! The first word of a program under test that is a bare name, found by the walk of `PATH` as a run walks it from
//! the copy: a program of the host (the file is in the settings digest of a simulation and of a run of examples, and
//! the copy is held to it), or, found under a relative entry, a file of the copy (it is not).
//!
//! `PATH` is the process's, and a test that sets it would race with every other test of the process that starts a
//! program. So this binary has this one test, and it sets `PATH` before anything else runs.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use chidev_core::app::ports::measure::{
    ExamplesRequest, InterfaceText, Runner, RunnerError, SimulateRequest,
};
use chidev_core::domain::vo::example::{Example, ExampleName, Verdict};
use chidev_core::domain::vo::measure::{FailureKind, SettingsDigest};
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::infra::project::{self, FILE};
use chidev_core::infra::runner::{self, ProcessRunner};

const TABLE: &str = r#"[measure]
build = ["sh", "-c", "true"]
env = {}

[measure.sim.ctx]
interpreter = ["sh"]
program = ["HOST", "{tree}/prog.sh"]

[measure.examples.ctx]
program = ["HOST", "{tree}/prog.sh"]

[[measure.layout]]
context = "ctx"
interface = "prog"
path = "prog.sh"

[[measure.layout]]
context = "ctx"
harness = "sim"
path = "script.sh"
"#;

/// A script that finds nothing and writes what a case has to.
const SCRIPT: &str =
    "printf s > \"$3/schedule\"\nprintf t > \"$3/trace\"\necho 'sim: findings=0'\n";

fn executable(dir: &Path, name: &str, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), text).unwrap();
    std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn runner_of(root: &Path, program: &str) -> ProcessRunner {
    std::fs::write(root.join(FILE), TABLE.replace("HOST", program)).unwrap();
    runner::load(&project::find(root).unwrap())
        .unwrap()
        .expect("a [measure] table")
}

fn names() -> (ContextName, AssetName, [AssetName; 1]) {
    (
        ContextName::try_from("ctx").unwrap(),
        AssetName::try_from("sim").unwrap(),
        [AssetName::try_from("prog").unwrap()],
    )
}

/// The digests of the settings of a simulation and of a run of examples of the Context `ctx`.
async fn digests(runner: &ProcessRunner) -> Result<(SettingsDigest, SettingsDigest), RunnerError> {
    let (ctx, harness, built) = names();
    Ok((
        runner.sim_settings(&ctx, &harness, &built).await?,
        runner.examples_settings(&ctx, &harness, &built).await?,
    ))
}

fn interfaces() -> Vec<InterfaceText> {
    vec![InterfaceText {
        name: names().2[0].clone(),
        code: "pinned\n".to_string(),
    }]
}

/// A simulation of `ctx` and a run of one example, each under the run name `run`: what each failed with.
async fn runs(runner: &ProcessRunner, run: &str) -> (Option<FailureKind>, Option<FailureKind>) {
    let (ctx, harness, _) = names();
    let sim = runner
        .simulate(&SimulateRequest {
            run: format!("sim-{run}"),
            context: ctx.clone(),
            harness: harness.clone(),
            script: SCRIPT.to_string(),
            interfaces: interfaces(),
            clauses: Vec::new(),
            regress: Vec::new(),
        })
        .await
        .unwrap();
    let example = Example::new(
        String::new(),
        "hi\n".to_string(),
        "hi\n#exit 0\n".to_string(),
        None,
    )
    .unwrap();
    let examples = runner
        .run_examples(&ExamplesRequest {
            run: format!("ex-{run}"),
            context: ctx,
            harness,
            interfaces: interfaces(),
            examples: vec![(ExampleName::new("a1").unwrap(), example)],
        })
        .await
        .unwrap();
    if examples.failure.is_none() {
        assert_eq!(examples.examples[0].1, Verdict::Ok, "{examples:#?}");
    }
    (
        sim.failure.map(|f| f.kind()),
        examples.failure.map(|f| f.kind()),
    )
}

#[test]
fn a_bare_name_is_walked_along_path_from_the_copy_and_is_the_hosts_unless_a_relative_entry_has_it()
{
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("chidev-program-on-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, host): (PathBuf, PathBuf) = (base.join("project"), base.join("host"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("prog.sh"), "").unwrap();
    let pid = std::process::id();
    let (on_path, in_project, runs_on_path) = (
        format!("chidev-on-path-{pid}"),
        format!("chidev-in-project-{pid}"),
        format!("chidev-runs-on-path-{pid}"),
    );
    executable(&host, &on_path, "#!/bin/sh\necho one\n");
    executable(&host, &runs_on_path, "#!/bin/sh\ncat\n");
    executable(&root.join("tools"), &in_project, "#!/bin/sh\necho one\n");
    // `../../shared` is a place from the root (`<tmp>/shared`, which is not there) and another from the copy, which
    // is `<root>/.chidev/measure/<run>/tree`: `<root>/.chidev/measure/shared`.
    let shared = root.join(".chidev/measure/shared");
    // SAFETY: this is the only test of the binary, and nothing has started a thread that reads the environment.
    unsafe {
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![PathBuf::from("../../shared")];
        paths.extend(std::env::split_paths(&old));
        paths.push(host.clone());
        paths.push(PathBuf::from("tools"));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // a program of the host: its file moves the digest of both kinds of run
            let runner = runner_of(&root, &on_path);
            let (sim, examples) = digests(&runner).await.unwrap();
            executable(&host, &on_path, "#!/bin/sh\necho two\n");
            let (sim_after, examples_after) = digests(&runner).await.unwrap();
            assert_ne!(sim, sim_after);
            assert_ne!(examples, examples_after);
            // a program of the project, found through the relative entry `tools`: its file is not in either
            let runner = runner_of(&root, &in_project);
            let (sim, examples) = digests(&runner).await.unwrap();
            executable(&root.join("tools"), &in_project, "#!/bin/sh\necho two\n");
            assert_eq!(digests(&runner).await.unwrap(), (sim, examples));
            // a name that `PATH` finds nowhere: the runner starts the program of examples, so that is an error of
            // its settings; the script of a simulation starts its program, which may be no executable, so it is not
            let runner = runner_of(&root, "chidev-found-nowhere");
            let (ctx, harness, built) = names();
            runner.sim_settings(&ctx, &harness, &built).await.unwrap();
            let error = runner
                .examples_settings(&ctx, &harness, &built)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("the program of the examples: "),
                "{error}"
            );
            // a program of the host that the copy resolves to the same file runs
            let runner = runner_of(&root, &runs_on_path);
            assert_eq!(runs(&runner, "same").await, (None, None));
            // the copy has the name under a relative entry that comes before the host's in the walk from the copy:
            // it is not the file the settings were digested with, and nothing is started
            executable(&shared, &runs_on_path, "#!/bin/sh\necho elsewhere\n");
            let failed = runs(&runner, "moved").await;
            assert_eq!(
                failed,
                (
                    Some(FailureKind::SettingsMoved),
                    Some(FailureKind::SettingsMoved)
                )
            );
            for run in ["sim-moved", "ex-moved"] {
                assert!(!root.join(".chidev/measure").join(run).exists());
            }
        });
    let _ = std::fs::remove_dir_all(&base);
}
