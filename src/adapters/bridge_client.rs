//! OpenFab → agent-chat Bridge client (Phase 1).
//!
//! The Bridge is a small sidecar that wraps the agent-chat backend + Matrix (Palpo). It
//! absorbs the async↔blocking impedance: OpenFab speaks plain blocking HTTP, the Bridge
//! does the async Matrix/agent work. OpenFab dispatches a coding task into a Matrix room
//! where the agent-chat implementer does the work, then OpenFab collects the produced
//! files back and signs/gates them — OpenFab stays the source of truth (it drives).
//!
//! Contract (OpenFab side):
//!   POST {bridge}/tasks     {spec_ref,intent,target_dir,language,acceptance,decisions,
//!                            allow,deny,room}                       → {task_id}
//!   GET  {bridge}/tasks/{id} → {status, files:{path:content}, file_hashes:{path:sha256},
//!                               model, prompt, error?}
//!   POST {bridge}/post      {room,msg}                             → {ok}
//!
//! Trust (Phase 1-Trust): OpenFab can only sign bytes it can hash. The Bridge MUST return
//! bit-identical full file contents and the exact prompt; [`BridgeResult::verify_integrity`]
//! cross-checks the returned content against the Bridge's claimed per-file hashes before
//! OpenFab accepts and signs them.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::core::sha256_hex;
use crate::core::spec::TaskCard;

/// The project's existing code, gathered to ship to the implementer so it can *refactor in
/// place* instead of generating a fresh (colliding) project. `truncated` is set when the cap
/// was hit; `tree` always lists every source path so the implementer sees the full structure.
#[derive(Debug, Clone, Default)]
pub struct ExistingCode {
    pub files: BTreeMap<String, String>,
    pub tree: Vec<String>,
    pub truncated: bool,
}

// Kept well under agent-chat's ~100KB message-body limit (the mounted code rides inside one
// agent-chat message). Boundary-scoping (below) usually keeps this to a file or two anyway.
const MOUNT_MAX_FILES: usize = 25;
const MOUNT_MAX_BYTES: usize = 80 * 1024;

fn is_source_path(rel: &str) -> bool {
    // Skip build artifacts, vcs, vendored deps, and OpenFab's own bookkeeping.
    const SKIP_DIRS: &[&str] = &[
        ".git/",
        "target/",
        "node_modules/",
        ".openfab/",
        "provenance/",
        "dist/",
        "build/",
        ".work/",
        "vendor/",
        "__pycache__/",
        ".venv/",
    ];
    if SKIP_DIRS
        .iter()
        .any(|d| rel.starts_with(d) || rel.contains(&format!("/{d}")))
    {
        return false;
    }
    const EXTS: &[&str] = &[
        "rs", "py", "js", "ts", "jsx", "tsx", "go", "java", "rb", "c", "h", "cpp", "hpp", "cs",
        "toml", "json", "yaml", "yml", "md", "txt", "cfg", "ini", "mod", "sum", "lock", "sh",
    ];
    let lower = rel.to_ascii_lowercase();
    EXTS.iter().any(|e| lower.ends_with(&format!(".{e}")))
        || lower.ends_with("/dockerfile")
        || lower == "dockerfile"
        || lower == "makefile"
}

/// Whether `rel` is in scope to mount, given the spec's allowed-change paths. When `allow` is
/// non-empty we mount ONLY files the contract permits editing (small + targeted); empty `allow`
/// falls back to the whole source tree (capped).
fn in_allow_scope(rel: &str, allow: &[String]) -> bool {
    if allow.is_empty() {
        return true;
    }
    allow.iter().any(|a| {
        let a = a
            .trim()
            .trim_start_matches("./")
            .trim_end_matches("**")
            .trim_end_matches('/');
        !a.is_empty() && (rel == a || rel.starts_with(&format!("{a}/")))
    })
}

/// Walk `repo` and gather its existing source files (capped, scoped to `allow` paths when given),
/// so the implementer can refactor in place rather than regenerate a colliding project.
pub fn gather_existing_files(repo: &std::path::Path, allow: &[String]) -> ExistingCode {
    let mut out = ExistingCode::default();
    let mut stack = vec![repo.to_path_buf()];
    let mut total = 0usize;
    let mut paths: Vec<std::path::PathBuf> = vec![];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(repo)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                // prune skip-dirs early
                if is_source_path(&format!("{rel}/x.rs")) || rel.is_empty() {
                    stack.push(p);
                }
            } else if is_source_path(&rel) && in_allow_scope(&rel, allow) {
                paths.push(p);
            }
        }
    }
    paths.sort();
    for p in &paths {
        let rel = p
            .strip_prefix(repo)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        out.tree.push(rel.clone());
    }
    for p in &paths {
        let rel = p
            .strip_prefix(repo)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        if out.files.len() >= MOUNT_MAX_FILES || total >= MOUNT_MAX_BYTES {
            out.truncated = true;
            break;
        }
        if let Ok(content) = std::fs::read_to_string(p) {
            if total + content.len() > MOUNT_MAX_BYTES {
                out.truncated = true;
                continue;
            }
            total += content.len();
            out.files.insert(rel, content);
        }
    }
    out
}

/// Shared-workspace mode (`OPENFAB_AGENTCHAT_WORKSPACE=shared`): the implementer is on the same
/// machine, so instead of shipping file bytes over the Bridge (size-limited, and starves the
/// agent of context) we hand it the repo PATH. It reads the whole repo for context, edits the
/// allowed files in place, and OpenFab reads those bytes back off disk to hash + sign.
pub fn workspace_shared() -> bool {
    std::env::var("OPENFAB_AGENTCHAT_WORKSPACE").as_deref() == Ok("shared")
}

/// Build the POST /tasks payload from a task card. In shared-workspace mode the payload carries
/// the repo PATH (the agent reads the whole repo for context, edits in place). Otherwise it
/// mounts the contract-allowed files (bounded fallback for a remote agent).
pub fn build_task_payload(task: &TaskCard, room: &str) -> serde_json::Value {
    // The files the contract permits editing (Boundaries → "may modify: X"), surfaced to the
    // agent so it knows exactly what to touch instead of inferring from code.
    let allow: Vec<String> = task
        .assumptions
        .iter()
        .filter_map(|a| a.strip_prefix("may modify:").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    // The agreed requirements doc (from the coordinator conversation), if present — the rich
    // "what & why" briefing, so the worker agent is told the task directly.
    let requirements = std::fs::read_to_string(
        task.workdir
            .join("specs")
            .join(format!("{}.requirements.md", task.spec_id)),
    )
    .unwrap_or_default();
    let mut base = serde_json::json!({
        "spec_ref": task.spec_ref(),
        "intent": task.intent,
        "target_dir": task.target_dir,
        "language": task.language,
        "acceptance": task.acceptance.iter().map(|a| &a.check).collect::<Vec<_>>(),
        "assumptions": task.assumptions,
        "context": task.context,
        "room": room,
        "allow": allow,
        "requirements": requirements,
    });
    if workspace_shared() {
        base["mode"] = serde_json::json!("workspace");
        base["repo_path"] = serde_json::json!(task.workdir.to_string_lossy());
        return base;
    }
    // Fallback (remote agent): mount the Boundary-allowed files — bounded, but no whole-repo
    // context. Prefer workspace mode on the same machine.
    let existing = gather_existing_files(&task.workdir, &allow);
    base["mode"] = serde_json::json!(if existing.files.is_empty() {
        "greenfield"
    } else {
        "refactor"
    });
    base["existing_files"] = serde_json::json!(existing.files);
    base["existing_tree"] = serde_json::json!(existing.tree);
    base["existing_truncated"] = serde_json::json!(existing.truncated);
    base
}

/// Read the files the implementer reported changing (workspace mode) off disk into a result, so
/// OpenFab hashes + signs exactly the bytes now in the repo. Trust holds: OpenFab signs what's
/// on disk, regardless of who wrote it.
pub fn workspace_result(
    repo: &std::path::Path,
    changed_paths: &[String],
    model: &str,
) -> Result<BridgeResult> {
    let mut files = BTreeMap::new();
    let mut file_hashes = BTreeMap::new();
    for rel in changed_paths {
        let safe = rel.trim_start_matches("./");
        if safe.contains("..") || safe.starts_with('/') {
            bail!("workspace result path escapes the repo: {rel}");
        }
        let content = std::fs::read_to_string(repo.join(safe))
            .with_context(|| format!("reading workspace-changed file {rel}"))?;
        file_hashes.insert(safe.to_string(), sha256_hex(content.as_bytes()));
        files.insert(safe.to_string(), content);
    }
    if files.is_empty() {
        bail!("implementer reported no changed files in the shared workspace");
    }
    Ok(BridgeResult {
        status: "done".into(),
        files,
        file_hashes,
        model: model.to_string(),
        prompt: "(shared workspace — edited in place)".into(),
        error: None,
        changed_paths: changed_paths.to_vec(),
    })
}

/// A parsed Bridge `/tasks/{id}` response.
#[derive(Debug, Clone, Default)]
pub struct BridgeResult {
    pub status: String,
    pub files: BTreeMap<String, String>,
    /// Bridge-claimed per-file sha256 (for transit integrity verification).
    pub file_hashes: BTreeMap<String, String>,
    pub model: String,
    pub prompt: String,
    pub error: Option<String>,
    /// Workspace mode: the paths the implementer edited in place (OpenFab reads them off disk).
    pub changed_paths: Vec<String>,
}

impl BridgeResult {
    pub fn is_done(&self) -> bool {
        self.status == "done"
    }
    pub fn is_failed(&self) -> bool {
        self.status == "failed"
    }

    /// Parse the Bridge `/tasks/{id}` JSON.
    pub fn parse(json: &serde_json::Value) -> Result<BridgeResult> {
        let status = json
            .get("status")
            .and_then(|v| v.as_str())
            .context("bridge task response has no status")?
            .to_string();
        let to_map = |key: &str| -> BTreeMap<String, String> {
            json.get(key)
                .and_then(|v| v.as_object())
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(BridgeResult {
            status,
            files: to_map("files"),
            file_hashes: to_map("file_hashes"),
            model: json
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            prompt: json
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            error: json
                .get("error")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            changed_paths: json
                .get("changed_paths")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Trust gate: EVERY returned file must have a claimed hash that matches the sha256 of its
    /// content (no truncation/corruption in transit), and there must be at least one file.
    /// OpenFab signs exactly these verified bytes — so we iterate `files` (what gets written +
    /// signed), not `file_hashes`, to ensure no unverified file can slip through unhashed.
    pub fn verify_integrity(&self) -> Result<()> {
        if self.files.is_empty() {
            bail!("bridge returned no files for the task");
        }
        for (path, content) in &self.files {
            let claimed = self
                .file_hashes
                .get(path)
                .with_context(|| format!("bridge returned file {path} with no integrity hash"))?;
            let actual = sha256_hex(content.as_bytes());
            if &actual != claimed {
                bail!(
                    "bridge file integrity check failed for {path}: claimed {claimed}, got {actual}"
                );
            }
        }
        Ok(())
    }

    /// Convert the verified files into an LLM-style manifest for `write_manifest`.
    pub fn into_manifest(self) -> crate::adapters::llm_backend::Manifest {
        crate::adapters::llm_backend::Manifest {
            files: self.files.into_iter().collect(),
            notes: format!("via agent-chat bridge (model {})", self.model),
        }
    }
}

// ---- HTTP (curl-based; exercised live against a running Bridge — see the checklist) ----

fn curl_json(args: &[&str], context: &str) -> Result<serde_json::Value> {
    let out = std::process::Command::new("curl")
        .args(args)
        .output()
        .with_context(|| format!("{context}: invoking curl"))?;
    if !out.status.success() {
        bail!(
            "{context}: curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    serde_json::from_slice(&out.stdout).with_context(|| {
        format!(
            "{context}: bridge reply was not JSON:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// POST a task to the Bridge; returns the task id.
pub fn post_task(bridge: &str, task: &TaskCard, room: &str) -> Result<String> {
    let body = build_task_payload(task, room).to_string();
    let url = format!("{}/tasks", bridge.trim_end_matches('/'));
    let v = curl_json(
        &[
            "-sS",
            "--max-time",
            "30",
            "--connect-timeout",
            "5",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ],
        "bridge POST /tasks",
    )?;
    v.get("task_id")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .context("bridge POST /tasks did not return a task_id")
}

/// GET a task's current state from the Bridge.
pub fn get_task(bridge: &str, task_id: &str) -> Result<BridgeResult> {
    let url = format!("{}/tasks/{}", bridge.trim_end_matches('/'), task_id);
    let v = curl_json(
        &["-sS", "--max-time", "30", "--connect-timeout", "5", &url],
        "bridge GET /tasks/{id}",
    )?;
    BridgeResult::parse(&v)
}

/// Post a narration message (e.g. gate/provenance summary) into the room via the Bridge.
pub fn post_message(bridge: &str, room: &str, msg: &str) -> Result<()> {
    let body = serde_json::json!({ "room": room, "msg": msg }).to_string();
    let url = format!("{}/post", bridge.trim_end_matches('/'));
    curl_json(
        &[
            "-sS",
            "--max-time",
            "30",
            "--connect-timeout",
            "5",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ],
        "bridge POST /post",
    )?;
    Ok(())
}

/// Dispatch a task to the Bridge and poll until done/failed, returning the verified result.
/// Polling cadence/timeout are configurable via env (`OPENFAB_BRIDGE_POLL_SECS`,
/// `OPENFAB_BRIDGE_TIMEOUT_SECS`).
pub fn dispatch_and_wait(bridge: &str, task: &TaskCard, room: &str) -> Result<BridgeResult> {
    let task_id = post_task(bridge, task, room)?;
    let poll = Duration::from_secs(env_secs("OPENFAB_BRIDGE_POLL_SECS", 5));
    let timeout = Duration::from_secs(env_secs("OPENFAB_BRIDGE_TIMEOUT_SECS", 1800));
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let r = get_task(bridge, &task_id)?;
        if r.is_done() {
            // Workspace mode: the implementer edited the repo in place; read the bytes it
            // reported changing off disk (that's what OpenFab hashes + signs).
            if workspace_shared() {
                let res = workspace_result(&task.workdir, &r.changed_paths, &r.model)?;
                res.verify_integrity()?;
                return Ok(res);
            }
            r.verify_integrity()?;
            return Ok(r);
        }
        if r.is_failed() {
            bail!(
                "bridge task {task_id} failed: {}",
                r.error.unwrap_or_default()
            );
        }
        if std::time::Instant::now() >= deadline {
            bail!("bridge task {task_id} timed out after {timeout:?}");
        }
        std::thread::sleep(poll);
    }
}

fn env_secs(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Build the `POST /review` payload: the AI-pending scenarios + the code to review.
pub fn build_review_payload(
    spec_ref: &str,
    requests: &serde_json::Value,
    files: &BTreeMap<String, String>,
    room: &str,
) -> serde_json::Value {
    serde_json::json!({
        "spec_ref": spec_ref,
        "requests": requests,
        "files": files,
        "room": room,
    })
}

/// Dispatch an agent-spec AI review to the Bridge (→ the reviewer agent) and poll until the
/// reviewer returns its decisions. Returns the raw `decisions` JSON array.
pub fn review_and_wait(
    bridge: &str,
    spec_ref: &str,
    requests: &serde_json::Value,
    files: &BTreeMap<String, String>,
    room: &str,
) -> Result<serde_json::Value> {
    let body = build_review_payload(spec_ref, requests, files, room).to_string();
    let url = format!("{}/review", bridge.trim_end_matches('/'));
    let posted = curl_json(
        &[
            "-sS",
            "--max-time",
            "30",
            "--connect-timeout",
            "5",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ],
        "bridge POST /review",
    )?;
    let review_id = posted
        .get("review_id")
        .and_then(|v| v.as_str())
        .context("bridge POST /review did not return a review_id")?
        .to_string();

    let poll = Duration::from_secs(env_secs("OPENFAB_BRIDGE_POLL_SECS", 5));
    let timeout = Duration::from_secs(env_secs("OPENFAB_BRIDGE_TIMEOUT_SECS", 1800));
    let deadline = std::time::Instant::now() + timeout;
    let get_url = format!("{}/review/{}", bridge.trim_end_matches('/'), review_id);
    loop {
        let r = curl_json(
            &[
                "-sS",
                "--max-time",
                "30",
                "--connect-timeout",
                "5",
                &get_url,
            ],
            "bridge GET /review/{id}",
        )?;
        match r.get("status").and_then(|v| v.as_str()) {
            Some("done") => {
                return Ok(r.get("decisions").cloned().unwrap_or(serde_json::json!([])))
            }
            Some("failed") => bail!(
                "bridge review {review_id} failed: {}",
                r.get("error").and_then(|v| v.as_str()).unwrap_or("")
            ),
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            bail!("bridge review {review_id} timed out after {timeout:?}");
        }
        std::thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::spec::Acceptance;
    use std::path::PathBuf;

    // `OPENFAB_AGENTCHAT_WORKSPACE` is process-global; serialize the tests that depend on it so
    // a parallel run of one doesn't see the other's env mutation (was a flaky failure).
    static WORKSPACE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn task() -> TaskCard {
        TaskCard {
            id: "t1".into(),
            spec_id: "demo".into(),
            spec_version: 1,
            intent: "add two ints".into(),
            context: vec!["docs/x.md".into()],
            assumptions: vec!["decision: python only".into()],
            acceptance: vec![Acceptance {
                id: "a1".into(),
                check: "agent-spec test: app::test_add".into(),
                must_pass: true,
            }],
            target_dir: "app".into(),
            language: Some("python".into()),
            workdir: PathBuf::from("/tmp/x"),
        }
    }

    #[test]
    fn task_payload_carries_contract_and_room() {
        let p = build_task_payload(&task(), "!demoboard:localhost");
        assert_eq!(p["spec_ref"], "demo#v1");
        assert_eq!(p["room"], "!demoboard:localhost");
        assert_eq!(p["target_dir"], "app");
        assert_eq!(p["acceptance"][0], "agent-spec test: app::test_add");
        assert_eq!(p["assumptions"][0], "decision: python only");
    }

    #[test]
    fn test_gather_existing_files_mounts_source_excludes_junk() {
        let tmp = std::env::temp_dir().join(format!("of-mount-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::create_dir_all(tmp.join("target/debug")).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(tmp.join("src/lib.rs"), "pub fn f(){}\n").unwrap();
        std::fs::write(tmp.join("target/debug/junk.rs"), "// build artifact\n").unwrap();
        std::fs::write(tmp.join(".git/config"), "[core]\n").unwrap();

        let ec = gather_existing_files(&tmp, &[]);
        assert!(ec.files.contains_key("Cargo.toml"));
        assert!(ec.files.contains_key("src/lib.rs"));
        // build artifacts + vcs are excluded
        assert!(!ec.files.keys().any(|k| k.starts_with("target/")));
        assert!(!ec.files.keys().any(|k| k.starts_with(".git/")));
        assert!(ec.tree.contains(&"src/lib.rs".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_payload_refactor_mode_when_repo_has_code() {
        let _g = WORKSPACE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("OPENFAB_AGENTCHAT_WORKSPACE"); // ensure not in workspace mode
        let tmp = std::env::temp_dir().join(format!("of-mount2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(tmp.join("src/lib.rs"), "pub fn f(){}\n").unwrap();
        let mut t = task();
        t.workdir = tmp.clone();
        let p = build_task_payload(&t, "!r:localhost");
        assert_eq!(p["mode"], "refactor");
        assert_eq!(p["existing_files"]["src/lib.rs"], "pub fn f(){}\n");
        let _ = std::fs::remove_dir_all(&tmp);
        // empty/greenfield workdir → greenfield mode
        let p2 = build_task_payload(&task(), "!r:localhost"); // workdir /tmp/x (no code)
        assert_eq!(p2["mode"], "greenfield");
    }

    #[test]
    fn test_workspace_result_reads_changed_files_from_disk() {
        let tmp = std::env::temp_dir().join(format!("of-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(tmp.join("src/lib.rs"), "pub fn f()->i32{42}\n").unwrap();
        // OpenFab reads exactly the bytes the agent left on disk + hashes them
        let r = workspace_result(&tmp, &["src/lib.rs".to_string()], "claude").unwrap();
        assert!(r.is_done());
        assert_eq!(r.files["src/lib.rs"], "pub fn f()->i32{42}\n");
        r.verify_integrity()
            .expect("disk bytes hash-match their own hashes");
        assert_eq!(r.changed_paths, vec!["src/lib.rs".to_string()]);
        // path traversal is rejected
        assert!(workspace_result(&tmp, &["../escape".to_string()], "x").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_workspace_payload_sends_path_not_files() {
        let _g = WORKSPACE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("OPENFAB_AGENTCHAT_WORKSPACE", "shared");
        let p = build_task_payload(&task(), "!r:localhost");
        assert_eq!(p["mode"], "workspace");
        assert!(p["repo_path"].is_string());
        assert!(p.get("existing_files").is_none()); // no bytes shipped — just the path
        std::env::remove_var("OPENFAB_AGENTCHAT_WORKSPACE");
    }

    #[test]
    fn parses_done_result_with_files() {
        let j = serde_json::json!({
            "status": "done",
            "files": { "app/add.py": "print(1)\n" },
            "model": "claude-x",
            "prompt": "implement add"
        });
        let r = BridgeResult::parse(&j).unwrap();
        assert!(r.is_done());
        assert_eq!(r.files["app/add.py"], "print(1)\n");
        assert_eq!(r.model, "claude-x");
        assert_eq!(r.prompt, "implement add");
    }

    #[test]
    fn integrity_passes_when_hashes_match() {
        let content = "print(1)\n";
        let mut r = BridgeResult {
            status: "done".into(),
            ..Default::default()
        };
        r.files.insert("app/add.py".into(), content.into());
        r.file_hashes
            .insert("app/add.py".into(), sha256_hex(content.as_bytes()));
        assert!(r.verify_integrity().is_ok());
    }

    #[test]
    fn integrity_fails_on_tampered_content() {
        let mut r = BridgeResult {
            status: "done".into(),
            ..Default::default()
        };
        r.files.insert("app/add.py".into(), "EVIL\n".into());
        r.file_hashes
            .insert("app/add.py".into(), sha256_hex(b"print(1)\n"));
        assert!(r.verify_integrity().is_err());
    }

    #[test]
    fn integrity_fails_on_empty_files() {
        let r = BridgeResult {
            status: "done".into(),
            ..Default::default()
        };
        assert!(r.verify_integrity().is_err());
    }

    #[test]
    fn integrity_fails_when_claimed_file_missing() {
        let mut r = BridgeResult {
            status: "done".into(),
            ..Default::default()
        };
        r.files.insert("app/a.py".into(), "x".into());
        r.file_hashes.insert("app/missing.py".into(), "abc".into());
        assert!(r.verify_integrity().is_err());
    }

    #[test]
    fn integrity_fails_when_a_returned_file_has_no_hash() {
        // Trust gap: a file present in `files` but absent from `file_hashes` must NOT pass —
        // it would otherwise be written + signed without verification.
        let mut r = BridgeResult {
            status: "done".into(),
            ..Default::default()
        };
        let good = "print(1)\n";
        r.files.insert("app/ok.py".into(), good.into());
        r.file_hashes
            .insert("app/ok.py".into(), sha256_hex(good.as_bytes()));
        // an extra, UNHASHED file
        r.files.insert("app/evil.py".into(), "rm -rf /\n".into());
        assert!(r.verify_integrity().is_err());
    }
}
