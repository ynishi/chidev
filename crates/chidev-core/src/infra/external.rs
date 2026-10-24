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
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::app::ports::external::{
    ExecutionEvidence, ExternalAttempt, ExternalCall, ExternalExecutor, ExternalFailure,
    ExternalReply,
};
use crate::domain::check::external::ExternalRule;
use crate::domain::check::{Hook, ParamValue};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::RuleId;

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
            let reads = registration.reads;
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

impl ProcessExecutor {
    async fn resolve(&self, command: &str) -> Result<PathBuf, ExternalFailure> {
        let command_path = Path::new(command);
        let candidate = if command_path.is_absolute() {
            command_path.to_path_buf()
        } else if command_path.components().count() > 1 {
            self.root.join(command_path)
        } else {
            let search = std::env::var_os("PATH").unwrap_or_default();
            let mut found = None;
            for path in std::env::split_paths(&search) {
                let path = if path.is_absolute() {
                    path
                } else {
                    self.root.join(path)
                };
                let candidate = path.join(command);
                if tokio::fs::metadata(&candidate).await.is_ok_and(|m| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        m.is_file() && m.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    {
                        m.is_file()
                    }
                }) {
                    found = Some(candidate);
                    break;
                }
            }
            found.ok_or_else(|| {
                failure("spawn", format!("executable `{command}` not found in PATH"))
            })?
        };
        tokio::fs::canonicalize(candidate)
            .await
            .map_err(|e| failure("spawn", e.to_string()))
    }
}

async fn executable_digest(path: &Path) -> Result<String, ExternalFailure> {
    if !tokio::fs::metadata(path)
        .await
        .map_err(|e| failure("spawn", e.to_string()))?
        .is_file()
    {
        return Err(failure(
            "spawn",
            "the configured executable is not a regular file",
        ));
    }
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| failure("spawn", e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buffer)
            .await
            .map_err(|e| failure("transport", e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

async fn bounded_read(
    reader: impl AsyncRead + Unpin,
    limit: usize,
    pipe: &'static str,
) -> Result<Vec<u8>, ExternalFailure> {
    let mut bytes = Vec::new();
    bounded_read_into(reader, limit, pipe, &mut bytes).await?;
    Ok(bytes)
}

async fn bounded_read_into(
    reader: impl AsyncRead + Unpin,
    limit: usize,
    pipe: &'static str,
    bytes: &mut Vec<u8>,
) -> Result<(), ExternalFailure> {
    reader
        .take((limit + 1) as u64)
        .read_to_end(bytes)
        .await
        .map_err(|e| failure("transport", e.to_string()))?;
    if bytes.len() > limit {
        return Err(failure("limit", format!("{pipe} exceeded {limit} bytes")));
    }
    Ok(())
}

fn failure(kind: &str, message: impl Into<String>) -> ExternalFailure {
    ExternalFailure {
        kind: kind.into(),
        message: message.into(),
    }
}

// The process group is created by this adapter, never supplied by the checker. Killing that group
// prevents a timeout from leaving a descendant holding a pipe (or consuming resources) indefinitely.
#[cfg(unix)]
struct ProcessGroup {
    child: Option<tokio::process::Child>,
    pid: i32,
}

#[cfg(unix)]
impl ProcessGroup {
    fn stop(&mut self) {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        // SAFETY: `pid` is the positive process id returned by our child spawn; its negation
        // identifies precisely the new process group created for that child. No pointers cross FFI.
        // SIGKILL is 9 on the Unix platforms supported by Rust.
        unsafe {
            kill(-self.pid, 9);
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }

    async fn stop_and_reap(&mut self) {
        self.stop();
        if let Some(mut child) = self.child.take() {
            let _ = child.wait().await;
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        self.stop();
        if let Some(mut child) = self.child.take() {
            // Cancellation cannot await. On a running runtime, explicitly reap asynchronously;
            // otherwise Tokio's kill_on_drop child reaper handles the direct child at shutdown.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}

impl ExternalExecutor for ProcessExecutor {
    async fn execute(&self, rule: &RuleId, call: ExternalCall) -> ExternalAttempt {
        let mut evidence = ExecutionEvidence::default();
        let result = self.attempt(rule, call, &mut evidence).await;
        ExternalAttempt { evidence, result }
    }
}

impl ProcessExecutor {
    #[cfg(not(unix))]
    async fn attempt(
        &self,
        _: &RuleId,
        _: ExternalCall,
        _: &mut ExecutionEvidence,
    ) -> Result<ExternalReply, ExternalFailure> {
        Err(failure(
            "unsupported",
            "external process groups are supported only on Unix",
        ))
    }

    #[cfg(unix)]
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
        let deadline = tokio::time::Instant::now()
            .checked_add(spec.timeout)
            .ok_or_else(|| failure("timeout", "configured timeout exceeds the clock's range"))?;
        let timed_out = || {
            failure(
                "timeout",
                format!("exceeded {} seconds", spec.timeout.as_secs()),
            )
        };
        let executable = tokio::time::timeout_at(deadline, async {
            let executable = self.resolve(&spec.command).await?;
            evidence.command = Some(executable.to_string_lossy().into_owned());
            evidence.command_digest = Some(executable_digest(&executable).await?);
            Ok::<_, ExternalFailure>(executable)
        })
        .await
        .map_err(|_| timed_out())??;
        let input = serde_json::to_vec(&call).map_err(|e| failure("protocol", e.to_string()))?;
        if tokio::time::Instant::now() >= deadline {
            return Err(timed_out());
        }
        let mut command = tokio::process::Command::new(&executable);
        command
            .args(&spec.args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| failure("spawn", e.to_string()))?;
        let pid = child
            .id()
            .and_then(|p| i32::try_from(p).ok())
            .ok_or_else(|| failure("spawn", "child has no valid process id"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| failure("transport", "missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure("transport", "missing stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| failure("transport", "missing stderr"))?;
        let mut group = ProcessGroup {
            child: Some(child),
            pid,
        };
        let mut stderr_bytes = Vec::new();
        let operation = async {
            let write = async {
                let mut stdin = stdin;
                stdin
                    .write_all(&input)
                    .await
                    .map_err(|e| failure("transport", e.to_string()))?;
                stdin
                    .shutdown()
                    .await
                    .map_err(|e| failure("transport", e.to_string()))?;
                Ok::<_, ExternalFailure>(())
            };
            let wait = async {
                let status = group
                    .child
                    .as_mut()
                    .ok_or_else(|| failure("transport", "child missing before wait"))?
                    .wait()
                    .await
                    .map_err(|e| failure("transport", e.to_string()))?;
                evidence.exit_code = status.code();
                Ok::<_, ExternalFailure>(status)
            };
            tokio::try_join!(
                write,
                bounded_read(stdout, STDOUT_LIMIT, "stdout"),
                bounded_read_into(stderr, STDERR_LIMIT, "stderr", &mut stderr_bytes),
                wait
            )
        };
        let result = tokio::time::timeout_at(deadline, operation).await;
        // Stop descendants even after normal exit. A checker must not daemonize a subprocess.
        group.stop_and_reap().await;
        stderr_bytes.truncate(STDERR_LIMIT);
        evidence.stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
        let (_, stdout, (), status) = result.map_err(|_| timed_out())??;
        if !status.success() {
            return Err(failure("exit", format!("process exited with {status}")));
        }
        serde_json::from_slice(&stdout).map_err(|e| failure("protocol", e.to_string()))
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
            model: crate::app::ports::external::ExternalModelView {
                version: 1,
                contexts: vec![],
                change_requests: None,
            },
        }
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
        request
            .model
            .contexts
            .push(serde_json::json!({"source": "a".repeat(256 * 1024)}));
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
