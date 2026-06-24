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

/// Build the POST /tasks payload from a task card (the implementer is constrained by the
/// contract's intent, acceptance, decisions, and boundaries).
pub fn build_task_payload(task: &TaskCard, room: &str) -> serde_json::Value {
    // Decisions / boundaries were folded into the spec's assumptions by the agent-spec
    // adapter; pass them through verbatim so the room agent sees the full contract.
    serde_json::json!({
        "spec_ref": task.spec_ref(),
        "intent": task.intent,
        "target_dir": task.target_dir,
        "language": task.language,
        "acceptance": task.acceptance.iter().map(|a| &a.check).collect::<Vec<_>>(),
        "assumptions": task.assumptions,
        "context": task.context,
        "room": room,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::spec::Acceptance;
    use std::path::PathBuf;

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
