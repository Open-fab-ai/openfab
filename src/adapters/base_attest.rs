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
    results: RefCell<HashMap<String, RunResult>>,
}

impl AttestBase {
    pub fn new(policy: Policy) -> Self {
        AttestBase {
            policy,
            results: RefCell::new(HashMap::new()),
        }
    }
}

/// Recursively collect files under `dir`, skipping dotfiles/dirs (e.g. `.openfab`, `.git`).
/// Absolute paths; the caller derives the repo-relative path for the attestation.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') {
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
        let target = task.workdir.join(&task.target_dir);
        let mut files = vec![];
        collect_files(&target, &mut files)?;
        files.sort();
        if files.is_empty() {
            anyhow::bail!(
                "attest: no files found under '{}' — nothing to attest (did the factory write them, and are they committed?)",
                task.target_dir
            );
        }

        let mut changed = vec![];
        for abs in &files {
            let bytes = std::fs::read(abs).with_context(|| format!("read {}", abs.display()))?;
            let rel = abs
                .strip_prefix(&task.workdir)
                .unwrap_or(abs)
                .to_string_lossy()
                .replace('\\', "/");
            let lines = bytes.iter().filter(|&&b| b == b'\n').count().max(1);
            changed.push(ChangedFile {
                path: rel,
                lines,
                sha256: sha256_hex(&bytes),
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
