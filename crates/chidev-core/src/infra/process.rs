//! Running a program: the machinery the external checks and the measurement runner share. A program is spawned
//! without a shell, in a process group of its own, under one deadline, with a cap on what it may write to each pipe.
//! A timeout or a cancellation kills the whole group and reaps the direct child; so does a normal exit, as soon as
//! the direct child has exited, because a program must not leave a descendant holding a pipe or consuming resources:
//! what a descendant writes after that is not read. A program that exits without reading all of its standard input
//! is not a failure for that alone; whether its answer is acceptable is the caller's to judge, as for every answer.
//! Only Unix process groups are implemented; other platforms answer with an explicit unsupported-platform failure.
//!
//! What is run, and what to do with its output, is the caller's: this module knows no protocol. A caller hands in
//! a [`Spec`] and a [`Capture`] that fills in as the program runs, so what the program wrote and how it exited are
//! kept even when the run fails (a timeout still shows the diagnostics written so far).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;

/// Why a program could not be run to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// It could not be found or started.
    Spawn,
    /// The deadline passed.
    Timeout,
    /// A pipe held more than its cap.
    Limit,
    /// A pipe could not be read or written.
    Transport,
    /// This platform cannot run programs in process groups.
    Unsupported,
}

impl FailureKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            FailureKind::Spawn => "spawn",
            FailureKind::Timeout => "timeout",
            FailureKind::Limit => "limit",
            FailureKind::Transport => "transport",
            FailureKind::Unsupported => "unsupported",
        }
    }
}

/// A run that did not end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What kind of failure.
    pub kind: FailureKind,
    /// The detail.
    pub message: String,
}

fn failure(kind: FailureKind, message: impl Into<String>) -> Failure {
    Failure {
        kind,
        message: message.into(),
    }
}

/// What to do when a pipe holds more than its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    /// The run fails ([`FailureKind::Limit`]): the output is a protocol document and a cut one is wrong.
    Fail,
    /// The first bytes are kept and the rest is read and thrown away: the output is a log.
    Truncate,
}

/// A cap on one pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pipe {
    /// The most bytes kept.
    pub limit: usize,
    /// What happens beyond it.
    pub over: Over,
}

/// One program to run.
#[derive(Debug)]
pub struct Spec<'a> {
    /// The program, as [`resolve`] gives it.
    pub executable: &'a Path,
    /// Its arguments.
    pub args: &'a [String],
    /// The directory it runs in.
    pub cwd: &'a Path,
    /// Variables added to the environment it inherits.
    pub env: &'a [(String, String)],
    /// What it reads on standard input; nothing (a closed input) when none.
    pub stdin: Option<&'a [u8]>,
    /// When the run must be over, from resolution to the end of the last pipe.
    pub deadline: Instant,
    /// The cap on standard output.
    pub stdout: Pipe,
    /// The cap on standard error.
    pub stderr: Pipe,
}

/// What a run left, whether or not it ended.
#[derive(Debug, Default)]
pub struct Capture {
    /// The exit code, once the program has exited (a signal has none).
    pub exit_code: Option<i32>,
    /// What it wrote on standard output, within the cap.
    pub stdout: Vec<u8>,
    /// What it wrote on standard error, within the cap.
    pub stderr: Vec<u8>,
}

/// The deadline `after` from now, if the clock has room for it.
pub fn deadline_in(after: Duration) -> Result<Instant, Failure> {
    Instant::now().checked_add(after).ok_or_else(|| {
        failure(
            FailureKind::Timeout,
            "the timeout exceeds the clock's range",
        )
    })
}

/// The executable `command` names, with its symbolic links resolved: the file itself. A path is taken relative to
/// `root` (an absolute one stays); a bare name is searched for in `PATH`.
pub async fn resolve(root: &Path, command: &str) -> Result<PathBuf, Failure> {
    let found = locate(root, command).await?;
    tokio::fs::canonicalize(found)
        .await
        .map_err(|e| failure(FailureKind::Spawn, e.to_string()))
}

/// The executable `command` names, found as [`resolve`] finds it but *not* followed through its links. A program
/// that is a link to another (`cargo` and `rustc` are links to `rustup`, which picks what to run by the name it was
/// started as) has to be started by the link's name; [`file_digest`] still reads the file behind it.
pub async fn locate(root: &Path, command: &str) -> Result<PathBuf, Failure> {
    let search = std::env::var_os("PATH").unwrap_or_default();
    locate_in(root, command, &search).await
}

/// As [`locate`], with `search` in the place of `PATH`: the one rule, with the list of directories handed in.
pub(crate) async fn locate_in(
    root: &Path,
    command: &str,
    search: &OsStr,
) -> Result<PathBuf, Failure> {
    let command_path = Path::new(command);
    let candidate = if command_path.is_absolute() {
        command_path.to_path_buf()
    } else if command_path.components().count() > 1 {
        root.join(command_path)
    } else {
        search_path(root, command, search)
            .await?
            .map(|hit| hit.file)
            .ok_or_else(|| {
                failure(
                    FailureKind::Spawn,
                    format!("executable `{command}` not found in PATH"),
                )
            })?
    };
    // A path that names nothing is a failure to start, here and not later at the spawn.
    tokio::fs::metadata(&candidate)
        .await
        .map_err(|e| failure(FailureKind::Spawn, format!("{}: {e}", candidate.display())))?;
    Ok(candidate)
}

/// Where a bare name was found along a search path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    /// The file.
    pub file: PathBuf,
    /// Whether the entry of the search path that has it is a relative one, which is taken from `root`.
    pub relative: bool,
}

/// The first entry of `search`, in order, that has an executable file called `command`: the one walk of a `PATH`,
/// for a relative entry is taken from `root` (the copy for a run, the project's root for the settings). None when no
/// entry has it. An entry that has no such file, or has a name that is not a file or is no directory, is passed over.
/// So is any other error on the way (an entry that cannot be read, say), as `execvp` passes over it: it is
/// remembered, and the walk goes on, so a later entry that has the name is the hit. Only when no entry has it is the
/// first such error the failure of the walk, for the name might have been behind that entry.
pub(crate) async fn search_path(
    root: &Path,
    command: &str,
    search: &OsStr,
) -> Result<Option<Hit>, Failure> {
    let mut unreadable = None;
    for entry in std::env::split_paths(search) {
        let relative = !entry.is_absolute();
        let dir = if relative { root.join(&entry) } else { entry };
        let file = dir.join(command);
        let meta = match tokio::fs::metadata(&file).await {
            Ok(meta) => meta,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(e) => {
                unreadable.get_or_insert_with(|| {
                    failure(FailureKind::Spawn, format!("{}: {e}", file.display()))
                });
                continue;
            }
        };
        let executable = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.is_file() && meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                meta.is_file()
            }
        };
        if executable {
            return Ok(Some(Hit { file, relative }));
        }
    }
    unreadable.map_or(Ok(None), Err)
}

/// The SHA-256 of the executable's bytes, in lower-case hex. It identifies the file, not what it loads.
pub async fn file_digest(path: &Path) -> Result<String, Failure> {
    if !tokio::fs::metadata(path)
        .await
        .map_err(|e| failure(FailureKind::Spawn, e.to_string()))?
        .is_file()
    {
        return Err(failure(
            FailureKind::Spawn,
            "the configured executable is not a regular file",
        ));
    }
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| failure(FailureKind::Spawn, e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buffer)
            .await
            .map_err(|e| failure(FailureKind::Transport, e.to_string()))?;
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

async fn read_pipe(
    mut reader: impl AsyncRead + Unpin,
    pipe: Pipe,
    name: &'static str,
    bytes: &mut Vec<u8>,
) -> Result<(), Failure> {
    match pipe.over {
        Over::Fail => {
            reader
                .take((pipe.limit + 1) as u64)
                .read_to_end(bytes)
                .await
                .map_err(|e| failure(FailureKind::Transport, e.to_string()))?;
            if bytes.len() > pipe.limit {
                return Err(failure(
                    FailureKind::Limit,
                    format!("{name} exceeded {} bytes", pipe.limit),
                ));
            }
        }
        Over::Truncate => {
            (&mut reader)
                .take(pipe.limit as u64)
                .read_to_end(bytes)
                .await
                .map_err(|e| failure(FailureKind::Transport, e.to_string()))?;
            // The program must not block on a full pipe: read the rest and drop it.
            tokio::io::copy(&mut reader, &mut tokio::io::sink())
                .await
                .map_err(|e| failure(FailureKind::Transport, e.to_string()))?;
        }
    }
    Ok(())
}

// The process group is created here, never supplied by the program. Killing that group prevents a timeout from
// leaving a descendant holding a pipe (or consuming resources) indefinitely.
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

/// Runs the program of `spec` to its end, filling `capture` as it goes. Returns how it exited; whether that is
/// success is the caller's to judge. The process group is stopped and reaped whatever the outcome, and when the
/// future is dropped.
#[cfg(not(unix))]
pub async fn run(_: &Spec<'_>, _: &mut Capture) -> Result<ExitStatus, Failure> {
    Err(failure(
        FailureKind::Unsupported,
        "process groups are supported only on Unix",
    ))
}

/// Runs the program of `spec` to its end, filling `capture` as it goes. Returns how it exited; whether that is
/// success is the caller's to judge. The process group is stopped and reaped whatever the outcome, and when the
/// future is dropped.
#[cfg(unix)]
pub async fn run(spec: &Spec<'_>, capture: &mut Capture) -> Result<ExitStatus, Failure> {
    let timed_out = || failure(FailureKind::Timeout, "the deadline passed");
    if Instant::now() >= spec.deadline {
        return Err(timed_out());
    }
    let mut command = tokio::process::Command::new(spec.executable);
    command
        .args(spec.args)
        .current_dir(spec.cwd)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(if spec.stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| failure(FailureKind::Spawn, e.to_string()))?;
    let pid = child
        .id()
        .and_then(|p| i32::try_from(p).ok())
        .ok_or_else(|| failure(FailureKind::Spawn, "child has no valid process id"))?;
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failure(FailureKind::Transport, "missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| failure(FailureKind::Transport, "missing stderr"))?;
    let mut group = ProcessGroup {
        child: Some(child),
        pid,
    };
    let Capture {
        exit_code,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    } = capture;
    let operation = async {
        let write = async {
            if let (Some(mut stdin), Some(input)) = (stdin, spec.stdin) {
                // A program may stop reading its input and still answer (it read one line, or refused the call):
                // the pipe closing under the write is its choice, not a failure of the transport, and what it wrote
                // and how it exited are still read and judged by the caller.
                let unread = |e: std::io::Error| {
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        Ok(())
                    } else {
                        Err(failure(FailureKind::Transport, e.to_string()))
                    }
                };
                match stdin.write_all(input).await {
                    Ok(()) => stdin.shutdown().await.or_else(unread)?,
                    Err(e) => unread(e)?,
                }
            }
            Ok::<_, Failure>(())
        };
        let wait = async {
            let status = group
                .child
                .as_mut()
                .ok_or_else(|| failure(FailureKind::Transport, "child missing before wait"))?
                .wait()
                .await
                .map_err(|e| failure(FailureKind::Transport, e.to_string()))?;
            *exit_code = status.code();
            // The run ends when the program does: a descendant still holding a pipe is stopped now, so the pipes
            // reach their end, instead of keeping the run open until the deadline and reading as a timeout.
            group.stop();
            Ok::<_, Failure>(status)
        };
        tokio::try_join!(
            write,
            read_pipe(stdout, spec.stdout, "stdout", stdout_bytes),
            read_pipe(stderr, spec.stderr, "stderr", stderr_bytes),
            wait
        )
    };
    let result = tokio::time::timeout_at(spec.deadline, operation).await;
    // Stop descendants even after a normal exit. A program must not daemonize a subprocess.
    group.stop_and_reap().await;
    capture.stdout.truncate(spec.stdout.limit);
    capture.stderr.truncate(spec.stderr.limit);
    let (_, (), (), status) = result.map_err(|_| timed_out())??;
    Ok(status)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "chidev-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn sh(
        script: &str,
        env: &[(String, String)],
        stdout: Pipe,
    ) -> (Capture, Result<ExitStatus, Failure>) {
        let cwd = dir();
        let shell = resolve(&cwd, "sh").await.unwrap();
        let args = ["-c".to_string(), script.to_string()];
        let spec = Spec {
            executable: &shell,
            args: &args,
            cwd: &cwd,
            env,
            stdin: None,
            deadline: deadline_in(Duration::from_secs(10)).unwrap(),
            stdout,
            stderr: Pipe {
                limit: 1024,
                over: Over::Truncate,
            },
        };
        let mut capture = Capture::default();
        let result = run(&spec, &mut capture).await;
        let _ = std::fs::remove_dir_all(cwd);
        (capture, result)
    }

    #[tokio::test]
    async fn the_environment_is_added_to_and_the_exit_code_is_kept() {
        let env = [("CHIDEV_PROCESS_TEST".to_string(), "seen".to_string())];
        let pipe = Pipe {
            limit: 1024,
            over: Over::Fail,
        };
        let (capture, result) = sh("printf %s \"$CHIDEV_PROCESS_TEST\"; exit 3", &env, pipe).await;
        assert!(!result.unwrap().success());
        assert_eq!(capture.exit_code, Some(3));
        assert_eq!(capture.stdout, b"seen");
    }

    #[tokio::test]
    async fn a_truncating_pipe_keeps_the_start_and_lets_the_program_finish() {
        let pipe = Pipe {
            limit: 8,
            over: Over::Truncate,
        };
        let (capture, result) = sh("head -c 200000 /dev/zero | tr '\\0' x", &[], pipe).await;
        assert!(result.unwrap().success());
        assert_eq!(capture.stdout, b"xxxxxxxx");
    }

    #[tokio::test]
    async fn a_failing_pipe_and_a_late_program_are_failures_with_what_was_written() {
        let pipe = Pipe {
            limit: 8,
            over: Over::Fail,
        };
        let (_, result) = sh("head -c 100 /dev/zero", &[], pipe).await;
        assert_eq!(result.unwrap_err().kind, FailureKind::Limit);
        let cwd = dir();
        let shell = resolve(&cwd, "sh").await.unwrap();
        let args = ["-c".to_string(), "echo partial >&2; sleep 20".to_string()];
        let spec = Spec {
            executable: &shell,
            args: &args,
            cwd: &cwd,
            env: &[],
            stdin: None,
            deadline: deadline_in(Duration::from_secs(1)).unwrap(),
            stdout: pipe,
            stderr: pipe,
        };
        let mut capture = Capture::default();
        let started = std::time::Instant::now();
        let error = run(&spec, &mut capture).await.unwrap_err();
        assert_eq!(error.kind, FailureKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(capture.stderr, b"partial\n");
        let _ = std::fs::remove_dir_all(cwd);
    }

    /// Runs `script` with `stdin` on its standard input and a deadline of 5 s.
    async fn sh_with_input(script: &str, stdin: &[u8]) -> (Capture, Result<ExitStatus, Failure>) {
        let cwd = dir();
        let shell = resolve(&cwd, "sh").await.unwrap();
        let args = ["-c".to_string(), script.to_string()];
        let pipe = Pipe {
            limit: 1024,
            over: Over::Fail,
        };
        let spec = Spec {
            executable: &shell,
            args: &args,
            cwd: &cwd,
            env: &[],
            stdin: Some(stdin),
            deadline: deadline_in(Duration::from_secs(5)).unwrap(),
            stdout: pipe,
            stderr: pipe,
        };
        let mut capture = Capture::default();
        let result = run(&spec, &mut capture).await;
        let _ = std::fs::remove_dir_all(cwd);
        (capture, result)
    }

    #[tokio::test]
    async fn a_program_that_does_not_read_all_of_its_input_still_answers() {
        // far more than a pipe holds, so the write is still going when the program exits
        let input = vec![b'x'; 4 * 1024 * 1024];
        let (capture, result) = sh_with_input("echo answered; exit 4", &input).await;
        assert_eq!(result.unwrap().code(), Some(4));
        assert_eq!(capture.stdout, b"answered\n");
    }

    #[tokio::test]
    async fn the_run_ends_when_the_program_does_though_a_descendant_holds_a_pipe() {
        let started = std::time::Instant::now();
        let (capture, result) = sh_with_input("sleep 30 & echo done", b"").await;
        assert!(result.unwrap().success());
        assert_eq!(capture.stdout, b"done\n");
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
