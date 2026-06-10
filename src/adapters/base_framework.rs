//! `base_framework` — the four PRD reference bases behind one `BasePort` adapter:
//! **AgentScope · HiClaw · agent-chat · OpenHands**.
//!
//! They share the same dispatch contract, so one parameterized adapter implements all
//! four (R3 — don't duplicate four near-identical files); only their metadata differs
//! (id, native runtime env, capabilities, comms channel). This is the "swap the base"
//! surface: the entire OpenFab Core (provenance, signing, trust, verify) runs identically
//! whichever of these is selected.
//!
//! Honesty (R14): each base reports a `runtime_mode`. If its native runtime is configured
//! (its endpoint env var is set), OpenFab dispatches to it for real and the run is
//! `native`. Otherwise the task runs through OpenFab's LLM bridge (`llm_backend`) and the
//! run is `bridged` — recorded truthfully in provenance and shown in the UI. Nothing
//! pretends an external server is running when it isn't.

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

    /// Env var whose presence means the base's native runtime is reachable.
    pub fn native_env(&self) -> &'static str {
        match self {
            Framework::AgentScope => "OPENFAB_AGENTSCOPE_URL",
            Framework::HiClaw => "OPENFAB_HICLAW_URL",
            Framework::AgentChat => "OPENFAB_AGENTCHAT_URL",
            Framework::OpenHands => "OPENFAB_OPENHANDS_URL",
        }
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

pub struct FrameworkBase {
    fw: Framework,
    policy: Policy,
    /// Some(endpoint) when the native runtime is configured (→ runtime_mode = native).
    native_endpoint: Option<String>,
    results: RefCell<HashMap<String, RunResult>>,
    memory: RefCell<HashMap<String, Vec<u8>>>,
}

impl FrameworkBase {
    pub fn new(fw: Framework, policy: Policy) -> Self {
        let native_endpoint = std::env::var(fw.native_env())
            .ok()
            .filter(|s| !s.is_empty());
        FrameworkBase {
            fw,
            policy,
            native_endpoint,
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
            None => {
                let gen = llm_backend::generate_bridge(&prompt)?;
                (
                    gen.manifest,
                    gen.model,
                    "bridged",
                    format!("OpenFab LLM bridge ({})", gen.provider),
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
}
