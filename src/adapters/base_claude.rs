//! `base_claude` — the claude CLI as a coding-agent base (its native runtime).
//!
//! The headline base: a genuine LLM turns the spec's NL intent into working source. The
//! generation mechanics (prompt building, manifest parsing, the CLI call, file writing)
//! live in `llm_backend` so every base shares one robust path; this file is just the
//! `BasePort` wiring. Swapping bases changes only which adapter is constructed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::adapters::{llm_backend, sandbox};
use crate::core::spec::TaskCard;
use crate::core::trust::Policy;
use crate::ports::base::{BasePort, Capabilities, ExecResult, RunHandle, RunResult};

pub struct ClaudeBase {
    policy: Policy,
    results: RefCell<HashMap<String, RunResult>>,
    memory: RefCell<HashMap<String, Vec<u8>>>,
}

impl ClaudeBase {
    pub fn new(policy: Policy) -> Self {
        ClaudeBase {
            policy,
            results: RefCell::new(HashMap::new()),
            memory: RefCell::new(HashMap::new()),
        }
    }
}

impl BasePort for ClaudeBase {
    fn name(&self) -> &str {
        "claude-cli"
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
            id: format!("{}-claude", task.id),
        };
        let prompt = llm_backend::build_prompt(task);
        let gen = llm_backend::generate_claude(&prompt)?;
        let changed = llm_backend::write_manifest(&task.workdir, &gen.manifest)?;
        let result = RunResult {
            task_id: task.id.clone(),
            changed_files: changed,
            model: gen.model,
            prompt,
            log: format!(
                "claude-cli implemented spec '{}' via {} — {} file(s); notes: {}",
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
        eprintln!("[claude comms #{channel}] {msg}");
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
