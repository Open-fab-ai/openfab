//! `base_framework` — the four PRD reference bases behind one `BasePort` adapter:
//! **AgentScope · HiClaw · agent-chat · OpenHands**.
//!
//! They share the same dispatch contract, so one parameterized adapter implements all
//! four (R3 — don't duplicate four near-identical files); only their metadata differs
//! (id, native runtime env, capabilities, comms channel). This is the "swap the base"
//! surface: the entire OpenFab Core (provenance, signing, trust, verify) runs identically
//! whichever of these is selected.
//!
//! Honesty (R14): native is decided by a *live probe* of the resolved endpoint (built-in
//! localhost default, overridable per-env), not merely by an env var being set — so a base
//! launched after OpenFab boots is still detected. If the runtime answers, OpenFab
//! dispatches to it for real (`native`). If it doesn't, OpenFab does NOT silently substitute
//! its own LLM: `dispatch` refuses unless the user explicitly opted into the bridged
//! stand-in (`allow_bridged`), in which case the run is recorded truthfully as `bridged`.
//! Nothing pretends an external server is running when it isn't.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::adapters::{llm_backend, sandbox};
use crate::core::spec::TaskCard;
use crate::core::trust::Policy;
use crate::ports::base::{BasePort, Capabilities, ExecResult, RunHandle, RunResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    AgentScope,
    HiClaw,
    AgentChat,
    OpenHands,
}

impl Framework {
    pub fn all() -> [Framework; 4] {
        [
            Framework::AgentScope,
            Framework::HiClaw,
            Framework::AgentChat,
            Framework::OpenHands,
        ]
    }

    pub fn from_id(s: &str) -> Option<Framework> {
        match s {
            "agentscope" => Some(Framework::AgentScope),
            "hiclaw" => Some(Framework::HiClaw),
            "agent-chat" | "agentchat" => Some(Framework::AgentChat),
            "openhands" => Some(Framework::OpenHands),
            _ => None,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Framework::AgentScope => "agentscope",
            Framework::HiClaw => "hiclaw",
            Framework::AgentChat => "agent-chat",
            Framework::OpenHands => "openhands",
        }
    }

    pub fn display(&self) -> &'static str {
        match self {
            Framework::AgentScope => "AgentScope",
            Framework::HiClaw => "HiClaw (Matrix)",
            Framework::AgentChat => "agent-chat",
            Framework::OpenHands => "OpenHands",
        }
    }

    /// Env var that OVERRIDES the built-in default endpoint (for non-default deploys).
    pub fn native_env(&self) -> &'static str {
        match self {
            Framework::AgentScope => "OPENFAB_AGENTSCOPE_URL",
            Framework::HiClaw => "OPENFAB_HICLAW_URL",
            Framework::AgentChat => "OPENFAB_AGENTCHAT_URL",
            Framework::OpenHands => "OPENFAB_OPENHANDS_URL",
        }
    }

    /// The localhost endpoint each bundled native adapter listens on by default.
    /// `None` = no bundled adapter for this framework (native only via env override).
    /// Keep in sync with the integration servers under integrations/.
    pub fn default_endpoint(&self) -> Option<&'static str> {
        match self {
            Framework::AgentScope => Some("http://127.0.0.1:8731/dispatch"),
            Framework::HiClaw => Some("http://127.0.0.1:8751/dispatch"),
            Framework::AgentChat => Some("http://127.0.0.1:8741/dispatch"),
            Framework::OpenHands => None,
        }
    }

    /// Endpoint OpenFab will probe and dispatch to: env override, else built-in default.
    /// Deliberately does NOT depend on the OpenFab server's launch-time env beyond the
    /// optional override, so a base started *after* OpenFab is still detected live.
    pub fn resolve_endpoint(&self) -> Option<String> {
        std::env::var(self.native_env())
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| self.default_endpoint().map(|s| s.to_string()))
    }

    /// Capabilities each framework natively brings (so OpenFab fills only the gaps).
    pub fn capabilities(&self) -> Capabilities {
        match self {
            // AgentScope: engine + ReMe memory + its own sandbox.
            Framework::AgentScope => Capabilities {
                orchestrate: true,
                comms: false,
                memory: true,
                sandbox: true,
            },
            // HiClaw: Matrix collaboration (comms) + memory; no sandbox of its own.
            Framework::HiClaw => Capabilities {
                orchestrate: true,
                comms: true,
                memory: true,
                sandbox: false,
            },
            // agent-chat: comms-centric.
            Framework::AgentChat => Capabilities {
                orchestrate: true,
                comms: true,
                memory: false,
                sandbox: false,
            },
            // OpenHands: strong sandboxed execution.
            Framework::OpenHands => Capabilities {
                orchestrate: true,
                comms: false,
                memory: false,
                sandbox: true,
            },
        }
    }

    pub fn comms_channel(&self) -> &'static str {
        match self {
            Framework::HiClaw => "matrix",
            Framework::AgentChat => "agent-chat",
            _ => "openfab",
        }
    }
}

/// Liveness probe: a successful TCP/HTTP connect (any HTTP status) means the adapter
/// is up. The adapters expose a POST-only `/dispatch`, so a GET returns 405 — curl still
/// exits 0 on a completed request, which is exactly the "it's listening" signal we want.
/// A connection refused (process down) makes curl exit non-zero → not reachable.
pub fn endpoint_reachable(url: &str) -> bool {
    std::process::Command::new("curl")
        .args(["-sS", "-o", "/dev/null", "--max-time", "2", url])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub struct FrameworkBase {
    fw: Framework,
    policy: Policy,
    /// Some(endpoint) only when the native runtime is reachable *right now* (live probe).
    native_endpoint: Option<String>,
    /// When the native runtime is unreachable, bridge via OpenFab's own LLM only if the
    /// user explicitly opted in — otherwise dispatch refuses (no silent stand-in, R14).
    allow_bridged: bool,
    /// Optional per-run model override the base should generate with (sent in the native
    /// dispatch payload; also used by the bridged path). `None` → the base's own default.
    base_model: Option<String>,
    results: RefCell<HashMap<String, RunResult>>,
    memory: RefCell<HashMap<String, Vec<u8>>>,
}

impl FrameworkBase {
    /// Native unless the user explicitly allows the bridged stand-in.
    pub fn new(fw: Framework, policy: Policy) -> Self {
        Self::with_bridge(fw, policy, false, None)
    }

    pub fn with_bridge(
        fw: Framework,
        policy: Policy,
        allow_bridged: bool,
        base_model: Option<String>,
    ) -> Self {
        // Native is determined by a *live probe* of the resolved endpoint, not merely by
        // an env var being set — so a base launched after OpenFab boots is still detected.
        let native_endpoint = fw.resolve_endpoint().filter(|u| endpoint_reachable(u));
        FrameworkBase {
            fw,
            policy,
            native_endpoint,
            allow_bridged,
            base_model,
            results: RefCell::new(HashMap::new()),
            memory: RefCell::new(HashMap::new()),
        }
    }

    /// Attempt a native dispatch to the framework's endpoint (only when configured).
    /// Posts the task as JSON and expects a `{files:{…}}` manifest back.
    fn dispatch_native(
        &self,
        endpoint: &str,
        task: &TaskCard,
    ) -> Result<(llm_backend::Manifest, String)> {
        let payload = serde_json::json!({
            "intent": task.intent,
            "target_dir": task.target_dir,
            "language": task.language,
            "acceptance": task.acceptance.iter().map(|a| &a.check).collect::<Vec<_>>(),
            // Optional per-run model the base should generate with (null → base default).
            "model": self.base_model,
        })
        .to_string();
        let out = std::process::Command::new("curl")
            .args([
                "-sS",
                "-X",
                "POST",
                endpoint,
                "-H",
                "Content-Type: application/json",
                "-d",
                &payload,
            ])
            .output()
            .with_context(|| format!("native dispatch to {} at {endpoint}", self.fw.display()))?;
        if !out.status.success() {
            anyhow::bail!(
                "{} native runtime returned an error: {}",
                self.fw.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let manifest = llm_backend::parse_manifest(&String::from_utf8_lossy(&out.stdout))?;
        Ok((manifest, format!("{}-native", self.fw.id())))
    }
}

impl BasePort for FrameworkBase {
    fn name(&self) -> &str {
        self.fw.id()
    }

    fn runtime_mode(&self) -> &str {
        if self.native_endpoint.is_some() {
            "native"
        } else {
            "bridged"
        }
    }

    fn capabilities(&self) -> Capabilities {
        self.fw.capabilities()
    }

    fn dispatch(&self, task: &TaskCard) -> Result<RunHandle> {
        let handle = RunHandle {
            id: format!("{}-{}", task.id, self.fw.id()),
        };
        let prompt = llm_backend::build_prompt(task);

        let (manifest, model, mode, provider) = match &self.native_endpoint {
            Some(endpoint) => {
                let (m, model) = self.dispatch_native(endpoint, task)?;
                (
                    m,
                    model,
                    "native",
                    format!("{} native runtime", self.fw.display()),
                )
            }
            None if !self.allow_bridged => {
                // The user chose this base explicitly; its native runtime is not running
                // and bridging was not opted into. Refuse rather than silently substitute
                // OpenFab's own LLM wearing the base's name (R14).
                anyhow::bail!(
                    "base '{}' is not running: native endpoint {} is unreachable. \
                     Launch it (POST /api/base/{}/launch) or re-run with bridged enabled.",
                    self.fw.display(),
                    self.fw
                        .resolve_endpoint()
                        .unwrap_or_else(|| "(none)".into()),
                    self.fw.id(),
                );
            }
            None => {
                let gen = llm_backend::generate_bridge(&prompt, self.base_model.as_deref())?;
                (
                    gen.manifest,
                    gen.model,
                    "bridged",
                    format!(
                        "OpenFab LLM bridge — stand-in for {} (not the real base) ({})",
                        self.fw.display(),
                        gen.provider
                    ),
                )
            }
        };

        let changed = llm_backend::write_manifest(&task.workdir, &manifest)?;
        let result = RunResult {
            task_id: task.id.clone(),
            changed_files: changed,
            model,
            prompt,
            log: format!(
                "base {} [{}] implemented spec '{}' via {} — {} file(s); notes: {}",
                self.fw.display(),
                mode,
                task.spec_id,
                provider,
                manifest.files.len(),
                manifest.notes
            ),
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

    fn post(&self, channel: &str, msg: &str) -> Result<()> {
        // HiClaw/agent-chat would post to their collaboration channel (the Matrix room is
        // the audit trail when base = HiClaw). Without a live server we tag the channel.
        let ch = if channel == "openfab" {
            self.fw.comms_channel()
        } else {
            channel
        };
        eprintln!("[{} comms #{ch}] {msg}", self.fw.id());
        Ok(())
    }

    fn memory_get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.memory.borrow().get(key).cloned())
    }

    fn memory_put(&self, key: &str, val: &[u8]) -> Result<()> {
        self.memory
            .borrow_mut()
            .insert(key.to_string(), val.to_vec());
        Ok(())
    }

    fn run_sandboxed(&self, cmd: &[String], workdir: &Path) -> Result<ExecResult> {
        sandbox::exec_gated(&self.policy, cmd, workdir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_frameworks_have_distinct_ids() {
        let ids: Vec<&str> = Framework::all().iter().map(|f| f.id()).collect();
        assert_eq!(ids, vec!["agentscope", "hiclaw", "agent-chat", "openhands"]);
        for id in &ids {
            assert!(Framework::from_id(id).is_some());
        }
    }

    #[test]
    fn runtime_mode_is_bridged_without_native_env() {
        // No native endpoint configured → honest "bridged".
        let b = FrameworkBase::new(Framework::OpenHands, Policy::default());
        assert_eq!(b.runtime_mode(), "bridged");
        assert_eq!(b.name(), "openhands");
    }

    #[test]
    fn capabilities_differ_per_framework() {
        assert!(Framework::AgentScope.capabilities().sandbox);
        assert!(Framework::HiClaw.capabilities().comms);
        assert!(!Framework::AgentChat.capabilities().memory);
    }

    #[test]
    fn bundled_frameworks_have_a_default_endpoint() {
        // The three with integration servers expose a localhost default; OpenHands doesn't.
        assert!(Framework::AgentChat
            .default_endpoint()
            .unwrap()
            .contains("8741"));
        assert!(Framework::AgentScope
            .default_endpoint()
            .unwrap()
            .contains("8731"));
        assert!(Framework::HiClaw
            .default_endpoint()
            .unwrap()
            .contains("8751"));
        assert!(Framework::OpenHands.default_endpoint().is_none());
    }

    #[test]
    fn reachable_is_false_for_a_dead_port() {
        // Nothing listens on :1 — connect refused → not reachable.
        assert!(!endpoint_reachable("http://127.0.0.1:1/dispatch"));
    }

    #[test]
    fn dispatch_refuses_when_unreachable_and_bridging_not_allowed() {
        use crate::core::spec::TaskCard;
        // OpenHands has no bundled adapter and bridging is off → must refuse, not bridge.
        let base = FrameworkBase::with_bridge(Framework::OpenHands, Policy::default(), false, None);
        let tmp = tempfile::tempdir().unwrap();
        let card = TaskCard {
            id: "t1".into(),
            spec_id: "demo".into(),
            spec_version: 1,
            intent: "Build a thing".into(),
            context: vec![],
            assumptions: vec![],
            acceptance: vec![],
            target_dir: "app".into(),
            language: None,
            workdir: tmp.path().to_path_buf(),
        };
        let err = base.dispatch(&card).unwrap_err().to_string();
        assert!(
            err.contains("not running"),
            "should refuse with a clear message: {err}"
        );
    }
}
