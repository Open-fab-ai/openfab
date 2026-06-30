//! `base_attest` — a base that ATTESTS existing files instead of generating them.
//!
//! The enterprise case (docs/ENTERPRISE_QUICKSTART.md, "Path B"): the code already
//! exists on disk — produced by the team's own AI agent factory — and they just want
//! the signed proof. This base generates nothing: `dispatch` reads the files already
//! present under the spec's `target_dir`, computes their digests, and hands them to the
//! normal spec-cycle, which runs the acceptance contract in the sandbox, signs, and
//! gates exactly as for any other base (R3 — no duplicate sign/gate path).
//!
//! `runtime_mode` is reported honestly as "attested" (R14): OpenFab did not run an agent
//! here — it notarized pre-existing files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::adapters::sandbox;
use crate::core::sha256_hex;
use crate::core::spec::TaskCard;
use crate::core::trust::Policy;
use crate::ports::base::{BasePort, Capabilities, ChangedFile, ExecResult, RunHandle, RunResult};

pub struct AttestBase {
    policy: Policy,
    /// (repo-relative path, bytes) captured at construction — BEFORE the spec-cycle branches.
    /// The forge branches a new run from the repo's root commit and `git clean`s the worktree
    /// (so each generated app is self-contained); that would wipe the very files we attest if
    /// we read them later. So we snapshot them up front and restore them in `dispatch`.
    captured: Vec<(String, Vec<u8>)>,
    results: RefCell<HashMap<String, RunResult>>,
}

impl AttestBase {
    /// Snapshot the existing files under `repo/target_dir` NOW (before the cycle branches).
    /// Errors if there is nothing to attest — empty input must never become a vacuous pass (R14).
    pub fn capture(repo: &Path, target_dir: &str, policy: Policy) -> Result<Self> {
        let target = repo.join(target_dir);
        let mut files = vec![];
        collect_files(&target, &mut files)?;
        files.sort();
        let mut captured = vec![];
        for abs in &files {
            let bytes = std::fs::read(abs).with_context(|| format!("read {}", abs.display()))?;
            let rel = abs
                .strip_prefix(repo)
                .unwrap_or(abs)
                .to_string_lossy()
                .replace('\\', "/");
            captured.push((rel, bytes));
        }
        if captured.is_empty() {
            anyhow::bail!(
                "attest: no files found under '{target_dir}/' — nothing to attest (did the factory write them, and are they committed?)"
            );
        }
        Ok(AttestBase {
            policy,
            captured,
            results: RefCell::new(HashMap::new()),
        })
    }
}

/// Recursively collect regular files under `dir`, skipping dotfiles/dirs (e.g. `.openfab`,
/// `.git`) and symlinks (we must not read/sign bytes that live outside the repo — R14).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') || path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

impl BasePort for AttestBase {
    fn name(&self) -> &str {
        "attest"
    }

    fn runtime_mode(&self) -> &str {
        "attested"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            orchestrate: false,
            comms: false,
            memory: false,
            sandbox: true, // we run the acceptance contract via OpenFab's sandbox
        }
    }

    fn dispatch(&self, task: &TaskCard) -> Result<RunHandle> {
        let handle = RunHandle {
            id: format!("{}-attest", task.id),
        };
        // The cycle branched from the root commit and cleaned the worktree before calling us,
        // so restore the snapshot taken at construction. The spec-cycle then commits these on
        // the run branch, runs the acceptance contract against them, and signs — exactly the
        // path a generating base takes, except the "generated" bytes are the captured originals.
        let mut changed = vec![];
        for (rel, bytes) in &self.captured {
            let abs = task.workdir.join(rel);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("mkdir {}", parent.display()))?;
            }
            std::fs::write(&abs, bytes).with_context(|| format!("restore {}", abs.display()))?;
            let lines = bytes.iter().filter(|&&b| b == b'\n').count().max(1);
            changed.push(ChangedFile {
                path: rel.clone(),
                lines,
                sha256: sha256_hex(bytes),
            });
        }

        // A deterministic, content-bound description stands in for the "prompt": there was
        // no generation prompt, so we record what was attested (its sha256 enters the AI-BOM).
        let prompt = format!(
            "OpenFab attestation of pre-existing files for spec '{}' ({} file(s) under {}/)",
            task.spec_id,
            changed.len(),
            task.target_dir
        );
        let log = format!(
            "attested {} pre-existing file(s) under {}/ — no generation performed",
            changed.len(),
            task.target_dir
        );
        let result = RunResult {
            task_id: task.id.clone(),
            changed_files: changed,
            model: String::new(), // no model — these files were not generated here
            prompt,
            log,
            success: true,
        };
        self.results.borrow_mut().insert(handle.id.clone(), result);
        Ok(handle)
    }

    fn result(&self, h: &RunHandle) -> Result<RunResult> {
        self.results
            .borrow()
            .get(&h.id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no result for handle {}", h.id))
    }

    fn post(&self, _channel: &str, _msg: &str) -> Result<()> {
        Ok(())
    }

    fn memory_get(&self, _key: &str) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn memory_put(&self, _key: &str, _val: &[u8]) -> Result<()> {
        Ok(())
    }

    fn run_sandboxed(&self, cmd: &[String], workdir: &Path) -> Result<ExecResult> {
        sandbox::exec_gated(&self.policy, cmd, workdir)
    }
}
