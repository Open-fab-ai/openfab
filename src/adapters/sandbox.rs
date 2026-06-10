//! Shared sandbox execution for bases that don't bring their own (PRD §3: "OpenFab
//! fills gaps when a base lacks a capability — e.g. uses its own sandbox").
//!
//! Production runtime (PRD §5): Podman / gVisor container. v0.1 fallback when no
//! container runtime is available: a **policy-gated host subprocess** confined to the
//! task workdir. The gate (`core::trust::Policy::check_command`) refuses anything off
//! the allowlist or matching the denylist BEFORE execution — a real, if lighter,
//! guardrail. The chosen runtime is recorded so provenance never overstates isolation.
//!
//! Every command runs under a **timeout** (hard-killed on expiry) so a runaway or
//! blocking command — e.g. a user "Try it" command that launches a server — can never
//! wedge the cycle or the web server's request lock.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::core::trust::Policy;
use crate::ports::base::ExecResult;

/// Default timeout for acceptance/reproduce checks.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Shorter timeout for ad-hoc "Try the software" commands.
pub const TRY_TIMEOUT_SECS: u64 = 25;

/// Which sandbox actually ran the command (recorded honestly in logs/provenance).
pub fn runtime_label() -> &'static str {
    // Docker present AND daemon reachable would upgrade this; the daemon was off in the
    // build environment, so v0.1 demos run the gated-host fallback. We never claim
    // container isolation we didn't use (R14: honest about the control/environment).
    "gated-host-subprocess"
}

/// Run `cmd` in `workdir` after the policy gate approves it, with the default timeout.
pub fn exec_gated(policy: &Policy, cmd: &[String], workdir: &Path) -> Result<ExecResult> {
    exec_gated_timeout(policy, cmd, workdir, DEFAULT_TIMEOUT_SECS)
}

/// Run `cmd` in `workdir` after the policy gate approves it, hard-killed after `secs`.
pub fn exec_gated_timeout(
    policy: &Policy,
    cmd: &[String],
    workdir: &Path,
    secs: u64,
) -> Result<ExecResult> {
    policy
        .check_command(cmd)
        .context("sandbox policy refused the command")?;

    let (prog, args) = cmd.split_first().context("empty sandbox command")?;
    let mut command = Command::new(prog);
    command
        .args(args)
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Run in its own process group so a timeout can kill the whole group — including any
    // background server a check spawned — instead of leaking orphans.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning sandboxed command: {}", cmd.join(" ")))?;
    let pid = child.id();

    // Drain pipes on threads to avoid buffer deadlock.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        let _ = tx_out.send(s);
    });
    thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        let _ = tx_err.send(s);
    });

    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait().context("waiting on sandboxed command")? {
            Some(status) => {
                let stdout = rx_out.recv().unwrap_or_default();
                let stderr = rx_err.recv().unwrap_or_default();
                return Ok(ExecResult {
                    exit_code: status.code().unwrap_or(-1),
                    stdout,
                    stderr,
                });
            }
            None => {
                if Instant::now() >= deadline {
                    // Kill the whole process group (negative pid) + the process itself.
                    let _ = Command::new("sh")
                        .arg("-c")
                        .arg(format!(
                            "kill -9 -{pid} 2>/dev/null; kill -9 {pid} 2>/dev/null"
                        ))
                        .status();
                    let _ = child.wait();
                    let stdout = rx_out.recv().unwrap_or_default();
                    let stderr = rx_err.recv().unwrap_or_default();
                    // Non-zero exit + a clear note; callers treat this as a failed check.
                    return Ok(ExecResult {
                        exit_code: 124, // conventional timeout code
                        stdout,
                        stderr: format!(
                            "{stderr}\n[sandbox] command exceeded {secs}s timeout and was killed (e.g. a long-running server — run it outside the sandbox)"
                        ),
                    });
                }
                thread::sleep(Duration::from_millis(120));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_captures() {
        let p = Policy::default();
        let r = exec_gated(
            &p,
            &["bash".into(), "-c".into(), "echo hi".into()],
            Path::new("."),
        )
        .unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hi"));
    }

    #[test]
    fn times_out_a_hanging_command() {
        let p = Policy::default();
        let r = exec_gated_timeout(
            &p,
            &["bash".into(), "-c".into(), "sleep 30".into()],
            Path::new("."),
            1,
        )
        .unwrap();
        assert_eq!(r.exit_code, 124);
        assert!(r.stderr.contains("timeout"));
    }
}
