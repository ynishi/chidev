//! The first word of a program under test that is a bare name, found through `PATH`: a program of the host (the file
//! is in the settings digest of a simulation and of a run of examples) or, when `PATH` has a relative entry that
//! finds it inside the project, a file of the project (it is not).
//!
//! `PATH` is the process's, and a test that sets it would race with every other test of the process that starts a
//! program. So this binary has this one test, and it sets `PATH` before anything else runs.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use chidev_core::app::ports::measure::{Runner, RunnerError};
use chidev_core::domain::vo::measure::SettingsDigest;
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

/// The digests of the settings of a simulation and of a run of examples of the Context `ctx`.
async fn digests(runner: &ProcessRunner) -> Result<(SettingsDigest, SettingsDigest), RunnerError> {
    let (ctx, harness, built) = (
        ContextName::try_from("ctx").unwrap(),
        AssetName::try_from("sim").unwrap(),
        [AssetName::try_from("prog").unwrap()],
    );
    Ok((
        runner.sim_settings(&ctx, &harness, &built).await?,
        runner.examples_settings(&ctx, &harness, &built).await?,
    ))
}

#[test]
fn a_bare_name_is_a_program_of_the_host_unless_path_finds_it_inside_the_project() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("chidev-program-on-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, host): (PathBuf, PathBuf) = (base.join("project"), base.join("host"));
    std::fs::create_dir_all(&root).unwrap();
    let (on_path, in_project) = (
        format!("chidev-on-path-{}", std::process::id()),
        format!("chidev-in-project-{}", std::process::id()),
    );
    executable(&host, &on_path, "#!/bin/sh\necho one\n");
    executable(&root.join("tools"), &in_project, "#!/bin/sh\necho one\n");
    // The entries are added after the ones the process has: a program found through the others is found as before.
    // SAFETY: this is the only test of the binary, and nothing has started a thread that reads the environment.
    unsafe {
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<PathBuf> = std::env::split_paths(&old).collect();
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
            // a name that `PATH` finds nowhere is the settings' error, for the program of each kind of run
            let runner = runner_of(&root, "chidev-found-nowhere");
            let (ctx, harness, built) = (
                ContextName::try_from("ctx").unwrap(),
                AssetName::try_from("sim").unwrap(),
                [AssetName::try_from("prog").unwrap()],
            );
            let error = runner
                .sim_settings(&ctx, &harness, &built)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("the program of the simulation: "),
                "{error}"
            );
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
        });
    let _ = std::fs::remove_dir_all(&base);
}
