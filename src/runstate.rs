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

// --- multi-project registry (Phase 2 D: each project is its own repo/workspace) ---

/// A managed project: a name and the repo/workspace that holds its runs + maintainers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub repo: String,
}

/// Project names must be filesystem-safe identifiers (no separators / traversal).
pub fn valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name != "default"
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn projects_file(projects_dir: &Path) -> PathBuf {
    projects_dir.join("projects.json")
}

/// Load the project registry (empty if none yet).
pub fn load_projects(projects_dir: &Path) -> Result<Vec<Project>> {
    let path = projects_file(projects_dir);
    if !path.exists() {
        return Ok(vec![]);
    }
    serde_json::from_str(&std::fs::read_to_string(&path)?).context("parse projects registry")
}

/// Register a project (idempotent on name). `repo` is created if absent.
pub fn add_project(projects_dir: &Path, name: &str, repo: &Path) -> Result<Project> {
    if !valid_project_name(name) {
        anyhow::bail!("invalid project name '{name}' (use letters, digits, - or _)");
    }
    let mut list = load_projects(projects_dir)?;
    if let Some(p) = list.iter().find(|p| p.name == name) {
        return Ok(p.clone());
    }
    std::fs::create_dir_all(repo)
        .with_context(|| format!("creating project repo {}", repo.display()))?;
    let proj = Project {
        name: name.to_string(),
        repo: repo.to_string_lossy().to_string(),
    };
    list.push(proj.clone());
    std::fs::create_dir_all(projects_dir)?;
    std::fs::write(
        projects_file(projects_dir),
        serde_json::to_string_pretty(&list).context("serialize projects")?,
    )?;
    Ok(proj)
}

/// A Robrix room bound to a project — so a coordinator's finalized doc is ingested into the
/// right project (Phase 2.1 #3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomBinding {
    pub room: String,
    pub project: String,
}

fn room_bindings_file(projects_dir: &Path) -> PathBuf {
    projects_dir.join("room-bindings.json")
}

/// Load the room→project bindings (empty if none yet).
pub fn load_room_bindings(projects_dir: &Path) -> Result<Vec<RoomBinding>> {
    let path = room_bindings_file(projects_dir);
    if !path.exists() {
        return Ok(vec![]);
    }
    serde_json::from_str(&std::fs::read_to_string(&path)?).context("parse room bindings")
}

/// Bind a Matrix room to a project (idempotent on room; rebinding updates the project).
pub fn bind_room(projects_dir: &Path, room: &str, project: &str) -> Result<()> {
    let mut list = load_room_bindings(projects_dir)?;
    match list.iter_mut().find(|b| b.room == room) {
        Some(b) => b.project = project.to_string(),
        None => list.push(RoomBinding {
            room: room.to_string(),
            project: project.to_string(),
        }),
    }
    std::fs::create_dir_all(projects_dir)?;
    std::fs::write(
        room_bindings_file(projects_dir),
        serde_json::to_string_pretty(&list).context("serialize room bindings")?,
    )?;
    Ok(())
}

/// Where an isolated worktree for a project lives, and the branch it checks out.
pub fn worktree_path(projects_dir: &Path, name: &str) -> PathBuf {
    projects_dir.join(name)
}
pub fn worktree_branch(name: &str) -> String {
    format!("openfab/{name}")
}

/// Build the `git` args to create an isolated worktree (pure, unit-tested).
pub fn worktree_add_args<'a>(src: &'a str, dest: &'a str, branch: &'a str) -> Vec<&'a str> {
    vec!["-C", src, "worktree", "add", dest, "-b", branch]
}

/// Create an isolated git worktree off `source_repo` for `name`, under the projects dir, and
/// return its path. Self-hosting writes into this worktree (a clean, separate checkout) so
/// the user's live working tree is never touched. The branch `openfab/<name>` shares the
/// source repo's object DB, so commits remain mergeable back.
pub fn create_worktree(projects_dir: &Path, name: &str, source_repo: &Path) -> Result<PathBuf> {
    if !valid_project_name(name) {
        anyhow::bail!("invalid project name '{name}'");
    }
    let dest = worktree_path(projects_dir, name);
    if dest.exists() {
        return Ok(dest); // idempotent: reuse an existing worktree
    }
    std::fs::create_dir_all(projects_dir)?;
    let branch = worktree_branch(name);
    let src = source_repo.to_string_lossy().to_string();
    let dest_s = dest.to_string_lossy().to_string();
    let args = worktree_add_args(&src, &dest_s, &branch);
    let out = std::process::Command::new("git")
        .args(&args)
        .output()
        .context("running git worktree add")?;
    if !out.status.success() {
        anyhow::bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(dest)
}

/// Resolve the project a room is bound to (pure; `None` if unbound).
pub fn resolve_room_project(bindings: &[RoomBinding], room: &str) -> Option<String> {
    bindings
        .iter()
        .find(|b| b.room == room)
        .map(|b| b.project.clone())
}

/// Resolve which repo a request targets. `None`/"default" → the default repo; a registered
/// project name → its repo; an unknown name → error. Pure (no I/O) so it is unit-tested.
pub fn resolve_project_repo(
    registry: &[Project],
    name: Option<&str>,
    default_repo: &Path,
) -> Result<PathBuf> {
    match name {
        None | Some("") | Some("default") => Ok(default_repo.to_path_buf()),
        Some(n) => registry
            .iter()
            .find(|p| p.name == n)
            .map(|p| PathBuf::from(&p.repo))
            .ok_or_else(|| anyhow::anyhow!("unknown project '{n}'")),
    }
}

// --- maintainer allowlist (the pre-approved human signer set) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintainerEntry {
    pub name: String,
    pub did: String,
    /// Phase 2: the Matrix user id this maintainer signs as (e.g. "@alice:palpo"). When set,
    /// approving in a Robrix room maps to this maintainer's N-of-M sign-off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mxid: Option<String>,
}

fn maintainers_file(repo: &Path) -> PathBuf {
    allow_dir(repo).join("maintainers.json")
}

/// Resolve which maintainer a Matrix user is allowed to sign as. SECURITY: an mxid that is
/// not mapped to exactly one allowlisted maintainer is rejected — a bare `approve` from an
/// unmapped room member can never produce a signature. Pure (no I/O) so it is unit-tested.
pub fn resolve_signer<'a>(
    mxid: &str,
    entries: &'a [MaintainerEntry],
) -> Result<&'a MaintainerEntry> {
    let matches: Vec<&MaintainerEntry> = entries
        .iter()
        .filter(|m| m.mxid.as_deref() == Some(mxid))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => anyhow::bail!("matrix user '{mxid}' is not mapped to any maintainer — cannot sign"),
        _ => anyhow::bail!("matrix user '{mxid}' maps to multiple maintainers — ambiguous"),
    }
}

/// Map a Matrix user id onto an already-registered maintainer (must be allowlisted first).
pub fn map_identity(repo: &Path, mxid: &str, maintainer_name: &str) -> Result<()> {
    let mut list = load_maintainers(repo)?;
    let entry = list
        .iter_mut()
        .find(|m| m.name == maintainer_name)
        .ok_or_else(|| anyhow::anyhow!("maintainer '{maintainer_name}' is not registered"))?;
    entry.mxid = Some(mxid.to_string());
    std::fs::create_dir_all(allow_dir(repo))?;
    std::fs::write(
        maintainers_file(repo),
        serde_json::to_string_pretty(&list).context("serialize maintainers")?,
    )?;
    Ok(())
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
        mxid: None,
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

/// Per-maintainer sign-off credential — the sha256 of a passphrase the human chooses. Only the
/// hash is stored (gitignored with the rest of `.openfab/identity/`); the plaintext never is.
/// Name-based sign-off (CLI `--as` / API `{as}`) must present a matching passphrase, closing the
/// "any local process can sign as anyone" hole. The Matrix-verified relay path doesn't need it.
fn maintainer_cred_path(repo: &Path, name: &str) -> PathBuf {
    maintainer_seed_dir(repo).join(format!("{name}.cred"))
}

pub fn maintainer_cred_hash(repo: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(maintainer_cred_path(repo, name))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn set_maintainer_cred(repo: &Path, name: &str, passphrase: &str) -> Result<()> {
    let p = maintainer_cred_path(repo, name);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, crate::core::sha256_hex(passphrase.as_bytes()))
        .with_context(|| format!("writing sign-off credential for {name}"))?;
    Ok(())
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

#[cfg(test)]
mod identity_tests {
    use super::*;

    fn m(name: &str, mxid: Option<&str>) -> MaintainerEntry {
        MaintainerEntry {
            name: name.into(),
            did: format!("did:key:{name}"),
            mxid: mxid.map(String::from),
        }
    }

    #[test]
    fn test_resolve_signer_maps_known_mxid() {
        let list = vec![
            m("alice", Some("@alice:palpo")),
            m("bob", Some("@bob:palpo")),
        ];
        let got = resolve_signer("@alice:palpo", &list).unwrap();
        assert_eq!(got.name, "alice");
    }

    #[test]
    fn test_resolve_signer_rejects_unmapped_mxid() {
        let list = vec![m("alice", Some("@alice:palpo"))];
        assert!(resolve_signer("@mallory:palpo", &list).is_err());
    }

    #[test]
    fn test_resolve_signer_rejects_ambiguous_mxid() {
        let list = vec![m("alice", Some("@x:palpo")), m("bob", Some("@x:palpo"))];
        assert!(resolve_signer("@x:palpo", &list).is_err());
    }

    fn proj(name: &str, repo: &str) -> Project {
        Project {
            name: name.into(),
            repo: repo.into(),
        }
    }

    #[test]
    fn test_resolve_project_repo_default() {
        let reg = vec![proj("alpha", "/ws/alpha")];
        let def = Path::new("/ws/default");
        assert_eq!(resolve_project_repo(&reg, None, def).unwrap(), def);
        assert_eq!(
            resolve_project_repo(&reg, Some("default"), def).unwrap(),
            def
        );
        assert_eq!(resolve_project_repo(&reg, Some(""), def).unwrap(), def);
    }

    #[test]
    fn test_resolve_project_repo_registered() {
        let reg = vec![proj("alpha", "/ws/alpha"), proj("beta", "/ws/beta")];
        let got = resolve_project_repo(&reg, Some("beta"), Path::new("/ws/default")).unwrap();
        assert_eq!(got, PathBuf::from("/ws/beta"));
    }

    #[test]
    fn test_resolve_project_repo_unknown() {
        let reg = vec![proj("alpha", "/ws/alpha")];
        assert!(resolve_project_repo(&reg, Some("ghost"), Path::new("/ws/default")).is_err());
    }

    #[test]
    fn test_worktree_add_command() {
        assert_eq!(
            worktree_add_args("/src/repo", "/wt/alpha", "openfab/alpha"),
            vec![
                "-C",
                "/src/repo",
                "worktree",
                "add",
                "/wt/alpha",
                "-b",
                "openfab/alpha"
            ]
        );
    }

    #[test]
    fn test_worktree_paths() {
        let pd = Path::new("/ws/projects");
        assert_eq!(
            worktree_path(pd, "selfdev"),
            PathBuf::from("/ws/projects/selfdev")
        );
        assert_eq!(worktree_branch("selfdev"), "openfab/selfdev");
    }

    #[test]
    fn test_resolve_room_project_bound() {
        let b = vec![RoomBinding {
            room: "!demoboard:palpo".into(),
            project: "alpha".into(),
        }];
        assert_eq!(
            resolve_room_project(&b, "!demoboard:palpo").as_deref(),
            Some("alpha")
        );
    }

    #[test]
    fn test_resolve_room_project_unbound() {
        let b = vec![RoomBinding {
            room: "!demoboard:palpo".into(),
            project: "alpha".into(),
        }];
        assert_eq!(resolve_room_project(&b, "!ghost:palpo"), None);
    }

    #[test]
    fn test_valid_project_name_rejects_traversal() {
        assert!(valid_project_name("alpha-1_x"));
        assert!(!valid_project_name("../etc"));
        assert!(!valid_project_name("a/b"));
        assert!(!valid_project_name(".."));
        assert!(!valid_project_name("default")); // reserved
        assert!(!valid_project_name(""));
    }
}
