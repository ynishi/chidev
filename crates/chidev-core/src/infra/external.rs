//! Project-registered checks executed as programs, without a shell. Registration is explicit in
//! `checks.toml`; the program has the caller's permissions, not a sandbox. Input selection limits the
//! JSON sent to it, not its access to the machine. Each attempt has its own process group on Unix.
//!
//! The process receives one [`ExternalCall`] JSON document on stdin and returns one [`ExternalReply`]
//! JSON document on stdout (protocol version 1). Stderr is diagnostics, never part of the response.
//! Stdout is capped at 1 MiB and stderr at 64 KiB; exceeding either cap fails the attempt. The default
//! timeout is 10 seconds and covers executable resolution and hashing as well as pipe I/O and process
//! completion. A timeout or cancellation kills the process group and reaps the direct child. Only Unix
//! process groups are implemented; other platforms return an explicit unsupported-platform failure.
//!
//! ```toml
//! [external."ext.example"]
//! command = "./tools/check-example"
//! args = []
//! hooks = ["put", "check"]
//! reads = ["contracts"]
//! timeout = 10
//! gate = false
//! params = { strict = true }
//! ```
//!
//! Commands containing a path are resolved relative to the project root (absolute paths stay absolute).
//! A bare executable name is resolved through PATH. SHA-256 records that executable's bytes before
//! spawning it; it does not fingerprint interpreter dependencies or attest against concurrent changes.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use super::process;
use crate::app::ports::external::{
    ExecutionEvidence, ExternalAttempt, ExternalCall, ExternalExecutor, ExternalFailure,
    ExternalReply,
};
use crate::domain::check::external::ExternalRule;
use crate::domain::check::{Hook, ParamValue};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::RuleId;
use crate::domain::vo::model_section::ModelSection;

/// Maximum bytes accepted from the response pipe.
const STDOUT_LIMIT: usize = 1024 * 1024;
/// Maximum bytes accepted from the diagnostics pipe.
const STDERR_LIMIT: usize = 64 * 1024;

/// Why an external registration could not be loaded. A malformed registration is not silently skipped.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}: {message}", path.display())]
pub struct ExternalConfigError {
    /// The configuration file.
    pub path: PathBuf,
    /// The read or validation error.
    pub message: String,
}

#[derive(Debug, Clone)]
struct ProcessSpec {
    command: String,
    args: Vec<String>,
    timeout: Duration,
    config_digest: String,
}

/// The process implementation of the application port. Its command map is fixed when configuration
/// is loaded; calls carry a rule id, never an arbitrary command to execute.
#[derive(Debug, Clone)]
pub struct ProcessExecutor {
    root: PathBuf,
    programs: BTreeMap<RuleId, ProcessSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    hooks: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout: u64,
    reads: Vec<String>,
    #[serde(default)]
    gate: bool,
    #[serde(default)]
    params: BTreeMap<String, toml::Value>,
}

fn default_timeout() -> u64 {
    10
}

/// Loads external checks from the project `checks.toml`. Other top-level tables are left to their
/// own readers. Rule order is lexicographic, independent of the file's table order.
pub fn load(root: &Path) -> Result<(Vec<ExternalRule>, ProcessExecutor), ExternalConfigError> {
    let path = root.join("checks.toml");
    let error = |message: String| ExternalConfigError {
        path: path.clone(),
        message,
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(error(e.to_string())),
    };
    let value: toml::Table = toml::from_str(&text).map_err(|e| error(e.to_string()))?;
    let mut rules = Vec::new();
    let mut programs = BTreeMap::new();
    if let Some(external) = value.get("external") {
        let table = external
            .as_table()
            .ok_or_else(|| error("`external` is a table of rule registrations".to_string()))?;
        for (name, entry) in table {
            let invalid = |message: String| error(format!("external.\"{name}\": {message}"));
            let registration: Registration = entry
                .clone()
                .try_into()
                .map_err(|e: toml::de::Error| invalid(e.to_string()))?;
            if registration.command.trim().is_empty() || registration.command.contains('\0') {
                return Err(invalid(
                    "`command` is a nonempty executable name without NUL".to_string(),
                ));
            }
            if registration.args.iter().any(|a| a.contains('\0')) {
                return Err(invalid("`args` cannot contain NUL".to_string()));
            }
            if registration.timeout == 0 {
                return Err(invalid("`timeout` must be positive seconds".to_string()));
            }
            let id = RuleId::try_from(name.as_str()).map_err(|e| invalid(e.to_string()))?;
            let hooks = registration
                .hooks
                .iter()
                .map(|h| Hook::from_word(h).ok_or_else(|| invalid(format!("unknown hook `{h}`"))))
                .collect::<Result<Vec<_>, _>>()?;
            let reads = registration
                .reads
                .iter()
                .map(|r| {
                    ModelSection::from_word(r)
                        .ok_or_else(|| invalid(format!("unknown section `{r}` in reads")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let params = registration
                .params
                .into_iter()
                .map(|(name, value)| {
                    let value = match value {
                        toml::Value::Boolean(v) => ParamValue::Bool(v),
                        toml::Value::Integer(v) => ParamValue::Int(v),
                        toml::Value::String(v) => ParamValue::Str(v),
                        _ => {
                            return Err(invalid(format!(
                                "params.{name} is not a scalar bool, integer or string"
                            )));
                        }
                    };
                    Ok((name, value))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            // Include rule metadata as well as the launch configuration: changing what a check reads
            // or when it runs must invalidate a saved observation too.
            let config_digest =
                Digest::of(&serde_json::to_vec(entry).map_err(|e| invalid(e.to_string()))?)
                    .to_string();
            let rule = ExternalRule::new(
                id.clone(),
                hooks,
                reads,
                registration.gate,
                config_digest.clone(),
            )
            .map_err(&invalid)?
            .with_params(params)
            .map_err(invalid)?;
            programs.insert(
                id,
                ProcessSpec {
                    command: registration.command,
                    args: registration.args,
                    timeout: Duration::from_secs(registration.timeout),
                    config_digest,
                },
            );
            rules.push(rule);
        }
    }
    let root = root.canonicalize().map_err(|e| error(e.to_string()))?;
    Ok((rules, ProcessExecutor { root, programs }))
}

fn failure(kind: &str, message: impl Into<String>) -> ExternalFailure {
    ExternalFailure {
        kind: kind.into(),
        message: message.into(),
    }
}

/// A process failure as an external check reports it: the kind by its word.
fn failed(f: process::Failure) -> ExternalFailure {
    failure(f.kind.word(), f.message)
}

impl ExternalExecutor for ProcessExecutor {
    async fn execute(&self, rule: &RuleId, call: ExternalCall) -> ExternalAttempt {
        let mut evidence = ExecutionEvidence::default();
        let result = self.attempt(rule, call, &mut evidence).await;
        ExternalAttempt { evidence, result }
    }
}

impl ProcessExecutor {
    async fn attempt(
        &self,
        rule: &RuleId,
        call: ExternalCall,
        evidence: &mut ExecutionEvidence,
    ) -> Result<ExternalReply, ExternalFailure> {
        let spec = self
            .programs
            .get(rule)
            .ok_or_else(|| failure("spawn", format!("no program configured for `{rule}`")))?;
        evidence.config_digest = spec.config_digest.clone();
        let deadline = process::deadline_in(spec.timeout).map_err(failed)?;
        let timed_out = || {
            failure(
                "timeout",
                format!("exceeded {} seconds", spec.timeout.as_secs()),
            )
        };
        let executable = tokio::time::timeout_at(deadline, async {
            let executable = process::resolve(&self.root, &spec.command)
                .await
                .map_err(failed)?;
            evidence.command = Some(executable.to_string_lossy().into_owned());
            evidence.command_digest =
                Some(process::file_digest(&executable).await.map_err(failed)?);
            Ok::<_, ExternalFailure>(executable)
        })
        .await
        .map_err(|_| timed_out())??;
        let input = serde_json::to_vec(&call).map_err(|e| failure("protocol", e.to_string()))?;
        let mut capture = process::Capture::default();
        let result = process::run(
            &process::Spec {
                executable: &executable,
                args: &spec.args,
                cwd: &self.root,
                env: &[],
                stdin: Some(&input),
                deadline,
                stdout: process::Pipe {
                    limit: STDOUT_LIMIT,
                    over: process::Over::Fail,
                },
                stderr: process::Pipe {
                    limit: STDERR_LIMIT,
                    over: process::Over::Fail,
                },
            },
            &mut capture,
        )
        .await;
        evidence.exit_code = capture.exit_code;
        evidence.stderr = String::from_utf8_lossy(&capture.stderr).into_owned();
        let status = result.map_err(|f| match f.kind {
            process::FailureKind::Timeout => timed_out(),
            _ => failed(f),
        })?;
        if !status.success() {
            return Err(failure("exit", format!("process exited with {status}")));
        }
        serde_json::from_slice(&capture.stdout).map_err(|e| failure("protocol", e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "chidev-external-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn config(&self, row: &str) {
            std::fs::write(
                self.0.join("checks.toml"),
                format!("[external.\"ext.test\"]\n{row}\n"),
            )
            .unwrap();
        }

        #[cfg(unix)]
        fn program(&self, body: &str) {
            use std::os::unix::fs::PermissionsExt;
            let path = self.0.join("checker");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const REGISTRATION: &str =
        "command = './checker'\nhooks = ['put', 'check']\nreads = ['contracts']\ntimeout = 10";

    fn call() -> ExternalCall {
        ExternalCall {
            protocol: 1,
            hook: "check".into(),
            rules: vec!["ext.test".into()],
            read_at: 0,
            params: BTreeMap::new(),
            model: crate::app::ports::external::ExternalModelView::empty(1),
        }
    }

    #[test]
    fn reads_keep_the_order_and_the_repeats_they_were_written_with() {
        use crate::domain::vo::model_section::ModelSection::*;
        let fixture = Fixture::new();
        fixture.config(&REGISTRATION.replace(
            "['contracts']",
            "['harnesses', 'contracts', 'harnesses', 'change_requests']",
        ));
        let (rules, _) = load(&fixture.0).unwrap();
        assert_eq!(
            rules[0].reads(),
            [Harnesses, Contracts, Harnesses, ChangeRequests]
        );
    }

    #[test]
    fn absent_registration_is_empty_and_bad_registration_is_refused() {
        let fixture = Fixture::new();
        assert!(load(&fixture.0).unwrap().0.is_empty());
        for (row, expected) in [
            (
                REGISTRATION.replace("timeout = 10", "timeout = 0"),
                "positive",
            ),
            (REGISTRATION.replace("'contracts'", "'unknown'"), "reads"),
            (
                REGISTRATION.replace("'contracts'", "'contract'"),
                "`contract`",
            ),
            (REGISTRATION.replace("'contracts'", ""), "reads"),
            (REGISTRATION.replace("'put', 'check'", "'unknown'"), "hook"),
            (format!("{REGISTRATION}\ntimeotu = 1"), "unknown field"),
            (
                format!("{REGISTRATION}\nparams = {{ limit = [1, 2] }}"),
                "scalar",
            ),
        ] {
            fixture.config(&row);
            let error = load(&fixture.0).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        fixture.config(REGISTRATION);
        let (rules, _) = load(&fixture.0).unwrap();
        assert_eq!(rules[0].descriptor().id.as_str(), "ext.test");
        assert!(!rules[0].gate());
    }

    #[test]
    fn scalar_parameter_defaults_join_the_existing_typed_policy() {
        use crate::domain::check::{Origin, Policy};
        let fixture = Fixture::new();
        fixture.config(&format!(
            "{REGISTRATION}\nparams = {{ limit = 3, strict = true, label = 'local' }}"
        ));
        let (rules, _) = load(&fixture.0).unwrap();
        let layer =
            crate::infra::policy::parse("[checks.\"ext.test\".params]\nlimit = 8", Origin::Project)
                .unwrap();
        let policy = Policy::resolve(&[rules[0].descriptor()], &[layer]).unwrap();
        let resolved = policy.of(&rules[0].descriptor().id).unwrap();
        assert_eq!(resolved.params.get("limit"), Some(&ParamValue::Int(8)));
        assert_eq!(resolved.params.get("strict"), Some(&ParamValue::Bool(true)));
        assert_eq!(
            resolved.params.get("label"),
            Some(&ParamValue::Str("local".into()))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_receives_json_and_returns_a_response_with_identity() {
        let fixture = Fixture::new();
        fixture.config(REGISTRATION);
        fixture.program("cat > received.json\necho diagnostic >&2\nprintf '%s' '{\"protocol\":1,\"findings\":[]}'");
        let (rules, executor) = load(&fixture.0).unwrap();
        let attempt = executor.execute(&rules[0].descriptor().id, call()).await;
        assert!(attempt.result.unwrap().findings.is_empty());
        assert_eq!(attempt.evidence.exit_code, Some(0));
        assert_eq!(attempt.evidence.stderr, "diagnostic\n");
        let bytes = std::fs::read(fixture.0.join("checker")).unwrap();
        assert_eq!(
            attempt.evidence.command_digest,
            Some(Digest::of(&bytes).to_string())
        );
        let received: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.0.join("received.json")).unwrap())
                .unwrap();
        assert_eq!(received["rules"], serde_json::json!(["ext.test"]));
        assert!(received["model"].get("change_requests").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn malformed_output_and_nonzero_exit_are_failures_not_empty_results() {
        let fixture = Fixture::new();
        fixture.config(REGISTRATION);
        let (rules, executor) = load(&fixture.0).unwrap();
        for (body, kind) in [
            ("cat >/dev/null; printf not-json", "protocol"),
            ("cat >/dev/null; echo failed >&2; exit 7", "exit"),
        ] {
            fixture.program(body);
            let attempt = executor.execute(&rules[0].descriptor().id, call()).await;
            assert_eq!(attempt.result.unwrap_err().kind, kind);
            if kind == "exit" {
                assert_eq!(attempt.evidence.exit_code, Some(7));
                assert_eq!(attempt.evidence.stderr, "failed\n");
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn oversized_output_and_a_child_holding_pipes_are_stopped() {
        let fixture = Fixture::new();
        fixture.config(REGISTRATION);
        let (rules, executor) = load(&fixture.0).unwrap();
        fixture.program("cat >/dev/null; head -c 1048577 /dev/zero");
        let attempt = executor.execute(&rules[0].descriptor().id, call()).await;
        assert_eq!(attempt.result.unwrap_err().kind, "limit");
        fixture.program("cat >/dev/null; head -c 65537 /dev/zero >&2");
        let attempt = executor.execute(&rules[0].descriptor().id, call()).await;
        assert_eq!(attempt.result.unwrap_err().kind, "limit");
        fixture.config(&REGISTRATION.replace("timeout = 10", "timeout = 3"));
        let (_, executor) = load(&fixture.0).unwrap();
        fixture.program("cat >/dev/null; (while [ ! -e release ]; do sleep 0.01; done; echo escaped > escaped) & echo ready > ready; wait");
        let attempt = executor.execute(&rules[0].descriptor().id, call()).await;
        assert_eq!(attempt.result.unwrap_err().kind, "timeout");
        std::fs::write(fixture.0.join("release"), "release").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!fixture.0.join("escaped").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_checker_that_never_reads_stdin_still_times_out() {
        let fixture = Fixture::new();
        fixture.config(&REGISTRATION.replace("timeout = 10", "timeout = 3"));
        fixture.program("echo waiting >&2; sleep 20");
        let (rules, executor) = load(&fixture.0).unwrap();
        let mut request = call();
        // more than a pipe holds, so the write blocks while the program never reads
        request
            .params
            .insert("pad".into(), serde_json::json!("a".repeat(256 * 1024)));
        let attempt = executor.execute(&rules[0].descriptor().id, request).await;
        assert_eq!(attempt.result.unwrap_err().kind, "timeout");
        assert_eq!(attempt.evidence.stderr, "waiting\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_an_attempt_stops_its_process_group() {
        let fixture = Fixture::new();
        fixture.config(REGISTRATION);
        fixture.program("cat >/dev/null; (while [ ! -e release ]; do sleep 0.01; done; echo escaped > escaped) & echo ready > ready; wait");
        let (rules, executor) = load(&fixture.0).unwrap();
        let rule = rules[0].descriptor().id.clone();
        let task = tokio::spawn(async move { executor.execute(&rule, call()).await });
        let file = fixture.0.join("ready");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !file.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        std::fs::write(fixture.0.join("release"), "release").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!fixture.0.join("escaped").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn normal_completion_also_stops_background_descendants() {
        let fixture = Fixture::new();
        fixture.config(REGISTRATION);
        fixture.program("cat >/dev/null; (while [ ! -e release ]; do sleep 0.01; done; echo escaped > escaped) >/dev/null 2>&1 & printf '%s' '{\"protocol\":1,\"findings\":[]}'");
        let (rules, executor) = load(&fixture.0).unwrap();
        let attempt = executor.execute(&rules[0].descriptor().id, call()).await;
        assert!(attempt.result.is_ok(), "{:?}", attempt.result);
        std::fs::write(fixture.0.join("release"), "release").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!fixture.0.join("escaped").exists());
    }
}
