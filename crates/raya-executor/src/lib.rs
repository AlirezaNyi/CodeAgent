//! Bounded process execution with timeout, output limits, and cancellation.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::debug;

/// Errors from the process executor.
#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to spawn process: {0}")]
    Spawn(std::io::Error),
}

pub type ExecutorResult<T> = Result<T, ExecutorError>;

/// Specification for a bounded process run.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Environment variables to set (merged over a cleared or inherited base).
    pub env: HashMap<String, String>,
    /// When true, clear inherited environment before applying `env`.
    pub clear_env: bool,
    pub timeout: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
    pub stdin: Option<Vec<u8>>,
}

impl ProcessSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            clear_env: false,
            timeout: Duration::from_secs(60),
            max_stdout: 256 * 1024,
            max_stderr: 256 * 1024,
            stdin: None,
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn max_stdout(mut self, n: usize) -> Self {
        self.max_stdout = n;
        self
    }

    pub fn max_stderr(mut self, n: usize) -> Self {
        self.max_stderr = n;
        self
    }
}

/// Result of a process execution.
#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub duration: Duration,
    pub cancelled: bool,
    pub timed_out: bool,
}

/// Run a process with timeout, output caps, and cancellation.
pub async fn run_process(
    spec: ProcessSpec,
    cancel: CancellationToken,
) -> ExecutorResult<ProcessOutput> {
    let start = Instant::now();

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .kill_on_drop(true);

    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    if spec.clear_env {
        command.env_clear();
    }
    for (k, v) in &spec.env {
        command.env(k, v);
    }

    debug!(
        program = %spec.program,
        args = ?spec.args,
        "spawning process"
    );

    let mut child = command.spawn().map_err(ExecutorError::Spawn)?;

    if let Some(data) = &spec.stdin
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(data).await?;
        stdin.shutdown().await?;
    }

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let max_out = spec.max_stdout;
    let max_err = spec.max_stderr;

    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut r) = stdout.take() {
            let mut tmp = [0u8; 8192];
            loop {
                match r.read(&mut tmp).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let remaining = max_out.saturating_sub(buf.len());
                        if remaining == 0 {
                            // Drain rest without storing
                            continue;
                        }
                        buf.extend_from_slice(&tmp[..n.min(remaining)]);
                    }
                    Err(_) => break,
                }
            }
        }
        buf
    });

    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut r) = stderr.take() {
            let mut tmp = [0u8; 8192];
            loop {
                match r.read(&mut tmp).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let remaining = max_err.saturating_sub(buf.len());
                        if remaining == 0 {
                            continue;
                        }
                        buf.extend_from_slice(&tmp[..n.min(remaining)]);
                    }
                    Err(_) => break,
                }
            }
        }
        buf
    });

    let mut timed_out = false;
    let mut cancelled = false;

    let status = tokio::select! {
        _ = cancel.cancelled() => {
            cancelled = true;
            let _ = child.start_kill();
            child.wait().await?
        }
        res = tokio::time::timeout(spec.timeout, child.wait()) => {
            match res {
                Ok(status) => status?,
                Err(_) => {
                    timed_out = true;
                    let _ = child.start_kill();
                    child.wait().await?
                }
            }
        }
    };

    let stdout_bytes = stdout_task.await.unwrap_or_else(|_| Vec::new());
    let stderr_bytes = stderr_task.await.unwrap_or_else(|_| Vec::new());

    let truncated = stdout_bytes.len() >= spec.max_stdout || stderr_bytes.len() >= spec.max_stderr;

    Ok(ProcessOutput {
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        truncated,
        duration: start.elapsed(),
        cancelled,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_ok() {
        let out = run_process(
            ProcessSpec::new("echo").args(["hello-raya"]),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.contains("hello-raya"));
        assert!(!out.timed_out);
        assert!(!out.cancelled);
    }

    #[tokio::test]
    async fn timeout_kills() {
        let out = run_process(
            ProcessSpec::new("sleep")
                .args(["5"])
                .timeout(Duration::from_millis(200)),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(out.timed_out);
        assert!(out.duration < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn cancel_kills() {
        let cancel = CancellationToken::new();
        let cancel2 = cancel.clone();
        let handle = tokio::spawn(async move {
            run_process(
                ProcessSpec::new("sleep")
                    .args(["5"])
                    .timeout(Duration::from_secs(30)),
                cancel2,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        let out = handle.await.unwrap().unwrap();
        assert!(out.cancelled);
    }

    #[tokio::test]
    async fn truncates_stdout() {
        // Print a long string
        let out = run_process(
            ProcessSpec::new("python3")
                .args(["-c", "print('x' * 10000)"])
                .max_stdout(100)
                .timeout(Duration::from_secs(10)),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        // python3 may not exist on all systems — fall back to yes|head via shell is complex;
        // if python missing, try printf.
        if out.exit_code != Some(0) {
            let out = run_process(
                ProcessSpec::new("sh")
                    .args(["-c", "printf '%0.s.' $(seq 1 5000)"])
                    .max_stdout(100)
                    .timeout(Duration::from_secs(10)),
                CancellationToken::new(),
            )
            .await
            .unwrap();
            assert!(out.stdout.len() <= 100);
            assert!(out.truncated);
        } else {
            assert!(out.stdout.len() <= 100);
            assert!(out.truncated);
        }
    }
}
