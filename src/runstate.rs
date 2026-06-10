//! Run state + maintainer allowlist persistence.
//!
//! This is the "decision memory" substrate (PRD §1: "the durable asset the fab
//! produces is the process + decision memory + signed provenance"). Each run persists
//! its spec, attestation pointer, acceptance outcomes, gate decision, and timeline so
//! that long-running, multi-session work survives — and so `signoff`/`verify` can act
//! on a run later. Maintainer identities + allowlist persist across invocations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::core::identity::{self, Identity};

pub fn openfab_dir(repo: &Path) -> PathBuf {
    repo.join(".openfab")
}
pub fn runs_dir(repo: &Path) -> PathBuf {
    openfab_dir(repo).join("runs")
}
pub fn run_dir(repo: &Path, id: &str) -> PathBuf {
    runs_dir(repo).join(id)
}
pub fn allow_dir(repo: &Path) -> PathBuf {
    openfab_dir(repo).join("allow")
}
pub fn fab_identity_dir(repo: &Path) -> PathBuf {
    identity::identity_dir(repo, "identity")
}
pub fn maintainer_seed_dir(repo: &Path) -> PathBuf {
    identity::identity_dir(repo, "maintainers")
}

fn default_gate_mode() -> String {
    "team".to_string()
}

/// Outcome of one acceptance criterion in the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceOutcome {
    pub id: String,
    pub check: String,
    pub passed: bool,
    pub exit_code: i32,
}

/// Everything needed to resume, sign off, or verify a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub spec_ref: String,
    pub base_name: String,
    /// "local" | "github" | "forgejo" | "gitea" | "gitcode" — the adapter kind, so the
    /// forge can be reconstructed without guessing from the (display) forge_name.
    #[serde(default)]
    pub forge_kind: String,
    pub forge_name: String,
    /// "native" | "bridged" | "deterministic" — how the base actually executed (R14: honest).
    #[serde(default)]
    pub base_runtime: String,
    /// "running" | "blocked" | "accepted" | "merged" | "failed" | "rejected".
    #[serde(default)]
    pub status: String,
    /// Human-approval gate mode: "solo" | "team" | "crowd" | "none" (see Policy::for_gate_mode).
    #[serde(default = "default_gate_mode")]
    pub gate_mode: String,
    pub branch: String,
    pub pr_url: String,
    /// Path of the committed attestation, relative to the repo root (portable).
    pub attestation_repo_path: String,
    pub sbom_repo_path: String,
    pub acceptance: Vec<AcceptanceOutcome>,
    pub acceptance_passed: bool,
    pub accepted: bool,
    pub merged: bool,
    pub parent_run: Option<String>,
    pub created: String,
}

impl RunRecord {
    /// Absolute path of the committed attestation.
    pub fn attestation_path(&self, repo: &Path) -> PathBuf {
        repo.join(&self.attestation_repo_path)
    }
}

/// One live timeline event (streamed to the web UI; mirrors a decision-log line).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub ts: String,
    pub icon: String,
    pub msg: String,
}

/// Lightweight run status, written progressively while a cycle runs in the background.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusFile {
    pub run_id: String,
    pub spec_ref: String,
    pub status: String,
    pub step: String,
    pub updated: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Append one event to a run's `events.jsonl` (best-effort; never panics the cycle).
pub fn append_event(repo: &Path, run_id: &str, ev: &Event) {
    let dir = run_dir(repo, run_id);
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(line) = serde_json::to_string(ev) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("events.jsonl"))
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

/// Read events for a run with `seq > since`.
pub fn read_events(repo: &Path, run_id: &str, since: u64) -> Vec<Event> {
    let path = run_dir(repo, run_id).join("events.jsonl");
    let mut out = vec![];
    if let Ok(text) = std::fs::read_to_string(&path) {
        for line in text.lines() {
            if let Ok(ev) = serde_json::from_str::<Event>(line) {
                if ev.seq > since {
                    out.push(ev);
                }
            }
        }
    }
    out
}

pub fn write_status(repo: &Path, st: &StatusFile) {
    let dir = run_dir(repo, &st.run_id);
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(j) = serde_json::to_string_pretty(st) {
            let _ = std::fs::write(dir.join("status.json"), j);
        }
    }
}

pub fn read_status(repo: &Path, run_id: &str) -> Option<StatusFile> {
    let path = run_dir(repo, run_id).join("status.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// Persist a run: `run.json`, the spec, and the timeline. The attestation itself lives
/// in the committed `provenance/` dir (pointed to by `attestation_repo_path`).
pub fn save_run(repo: &Path, rec: &RunRecord, spec_yaml: &str, timeline: &str) -> Result<()> {
    let dir = run_dir(repo, &rec.run_id);
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    std::fs::write(
        dir.join("run.json"),
        serde_json::to_string_pretty(rec).context("serialize run record")?,
    )?;
    std::fs::write(dir.join("spec.yaml"), spec_yaml)?;
    std::fs::write(dir.join("timeline.md"), timeline)?;
    Ok(())
}

pub fn load_run(repo: &Path, id: &str) -> Result<RunRecord> {
    let path = run_dir(repo, id).join("run.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no run '{id}' at {}", path.display()))?;
    serde_json::from_str(&text).context("parse run.json")
}

pub fn load_run_spec_yaml(repo: &Path, id: &str) -> Result<String> {
    let path = run_dir(repo, id).join("spec.yaml");
    std::fs::read_to_string(&path).with_context(|| format!("reading spec for run {id}"))
}

pub fn list_runs(repo: &Path) -> Result<Vec<RunRecord>> {
    let dir = runs_dir(repo);
    let mut out = vec![];
    if dir.exists() {
        for entry in std::fs::read_dir(&dir)? {
            let p = entry?.path();
            if p.join("run.json").exists() {
                if let Some(id) = p.file_name().and_then(|s| s.to_str()) {
                    if let Ok(r) = load_run(repo, id) {
                        out.push(r);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.created.cmp(&b.created));
    Ok(out)
}

// --- maintainer allowlist (the pre-approved human signer set) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintainerEntry {
    pub name: String,
    pub did: String,
}

fn maintainers_file(repo: &Path) -> PathBuf {
    allow_dir(repo).join("maintainers.json")
}

pub fn load_maintainers(repo: &Path) -> Result<Vec<MaintainerEntry>> {
    let path = maintainers_file(repo);
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text).context("parse maintainers allowlist")
}

pub fn maintainer_dids(repo: &Path) -> Result<Vec<String>> {
    Ok(load_maintainers(repo)?.into_iter().map(|m| m.did).collect())
}

/// Register a pre-approved maintainer: create/load their identity seed and add them to
/// the allowlist if absent. Returns (did, was_new).
pub fn add_maintainer(repo: &Path, name: &str) -> Result<(String, bool)> {
    let id = Identity::load_or_create(&maintainer_seed_dir(repo), name)?;
    let did = id.did();
    let mut list = load_maintainers(repo)?;
    if list.iter().any(|m| m.did == did) {
        return Ok((did, false));
    }
    list.push(MaintainerEntry {
        name: name.to_string(),
        did: did.clone(),
    });
    std::fs::create_dir_all(allow_dir(repo))?;
    std::fs::write(
        maintainers_file(repo),
        serde_json::to_string_pretty(&list).context("serialize maintainers")?,
    )?;
    Ok((did, true))
}

/// Load a maintainer's signing identity (must already be registered).
pub fn load_maintainer_identity(repo: &Path, name: &str) -> Result<Identity> {
    Identity::load_or_create(&maintainer_seed_dir(repo), name)
}

// --- fab allowlist (the trusted fab identities) ---

fn fab_allow_file(repo: &Path) -> PathBuf {
    allow_dir(repo).join("fab.json")
}

pub fn fab_allowlist(repo: &Path) -> Result<Vec<String>> {
    let path = fab_allow_file(repo);
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text).context("parse fab allowlist")
}

/// Ensure the fab DID is allowlisted (the repo trusts its own fab).
pub fn ensure_fab_allowlisted(repo: &Path, did: &str) -> Result<()> {
    let mut list = fab_allowlist(repo)?;
    if !list.iter().any(|d| d == did) {
        list.push(did.to_string());
        std::fs::create_dir_all(allow_dir(repo))?;
        std::fs::write(
            fab_allow_file(repo),
            serde_json::to_string_pretty(&list).context("serialize fab allowlist")?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintainer_registration_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let (did1, new1) = add_maintainer(repo, "alice").unwrap();
        let (did2, new2) = add_maintainer(repo, "alice").unwrap();
        assert_eq!(did1, did2);
        assert!(new1 && !new2);
        assert_eq!(maintainer_dids(repo).unwrap(), vec![did1]);
    }

    #[test]
    fn fab_allowlist_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        ensure_fab_allowlisted(repo, "did:key:zFAB").unwrap();
        ensure_fab_allowlisted(repo, "did:key:zFAB").unwrap();
        assert_eq!(
            fab_allowlist(repo).unwrap(),
            vec!["did:key:zFAB".to_string()]
        );
    }
}
