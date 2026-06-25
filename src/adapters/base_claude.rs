//! `base_claude` — a local coding-CLI as a base (its native runtime): **claude** or **codex**.
//!
//! The headline base: a genuine LLM turns the spec's NL intent into working source. Both CLIs
//! share one `BasePort` wiring (R3 — no near-duplicate adapters); only which `llm_backend`
//! generator runs differs. The generation mechanics (prompt, manifest parse, CLI call, file
//! writing) live in `llm_backend`. Swapping bases changes only which adapter is constructed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::adapters::{llm_backend, sandbox};
use crate::core::spec::TaskCard;
use crate::core::trust::Policy;
use crate::ports::base::{BasePort, Capabilities, ExecResult, RunHandle, RunResult};

/// Which local coding CLI backs this base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliKind {
    Claude,
    Codex,
}

impl CliKind {
    fn id(&self) -> &'static str {
        match self {
            CliKind::Claude => "claude-cli",
            CliKind::Codex => "codex-cli",
        }
    }

    fn generate(&self, prompt: &str) -> Result<llm_backend::GenOutput> {
        match self {
            CliKind::Claude => llm_backend::generate_claude(prompt),
            CliKind::Codex => llm_backend::generate_codex(prompt),
        }
    }
}

pub struct ClaudeBase {
    kind: CliKind,
    policy: Policy,
    results: RefCell<HashMap<String, RunResult>>,
    memory: RefCell<HashMap<String, Vec<u8>>>,
}

impl ClaudeBase {
    /// The claude CLI base (back-compat constructor).
    pub fn new(policy: Policy) -> Self {
        Self::with_kind(CliKind::Claude, policy)
    }

    pub fn with_kind(kind: CliKind, policy: Policy) -> Self {
        ClaudeBase {
            kind,
            policy,
            results: RefCell::new(HashMap::new()),
            memory: RefCell::new(HashMap::new()),
        }
    }
}

impl BasePort for ClaudeBase {
    fn name(&self) -> &str {
        self.kind.id()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            orchestrate: true,
            comms: false,
            memory: false,
            sandbox: false,
        }
    }

    fn dispatch(&self, task: &TaskCard) -> Result<RunHandle> {
        let handle = RunHandle {
            id: format!("{}-{}", task.id, self.kind.id()),
        };
        let prompt = llm_backend::build_prompt(task);
        let gen = self.kind.generate(&prompt)?;
        let changed = llm_backend::write_manifest(&task.workdir, &gen.manifest)?;
        let result = RunResult {
            task_id: task.id.clone(),
            changed_files: changed,
            model: gen.model,
            prompt,
            log: format!(
                "{} implemented spec '{}' via {} — {} file(s); notes: {}",
                self.kind.id(),
                task.spec_id,
                gen.provider,
                gen.manifest.files.len(),
                gen.manifest.notes
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
        eprintln!("[{} comms #{channel}] {msg}", self.kind.id());
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
