//! A base that *imports an already-built artifact* instead of generating one — e.g. code the
//! agent-chat team produced in a Robrix room. `dispatch` writes the provided files into the
//! repo; every later step of `run_cycle` (agent-spec verify, in-toto/SLSA signing, conformance,
//! N-of-M gate, PR) runs unchanged. This is how *any* build path — dashboard-driven or
//! room-driven — converges on the single OpenFab gate.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;

use crate::adapters::{llm_backend, sandbox};
use crate::core::spec::TaskCard;
use crate::core::trust::Policy;
use crate::ports::base::{BasePort, Capabilities, ExecResult, RunHandle, RunResult};

pub struct PrebuiltBase {
    label: String,
    model: String,
    manifest: llm_backend::Manifest,
    policy: Policy,
    results: RefCell<HashMap<String, RunResult>>,
    memory: RefCell<HashMap<String, Vec<u8>>>,
}

impl PrebuiltBase {
    /// `label` names the upstream builder (e.g. "agent-chat") for provenance; `model` is what it
    /// reported building with; `files` is the final artifact (relpath → content).
    pub fn new(
        label: impl Into<String>,
        model: impl Into<String>,
        files: BTreeMap<String, String>,
        policy: Policy,
    ) -> Self {
        let label = label.into();
        PrebuiltBase {
            manifest: llm_backend::Manifest {
                files,
                notes: format!("imported pre-built artifact from {label}"),
            },
            label,
            model: model.into(),
            policy,
            results: RefCell::new(HashMap::new()),
            memory: RefCell::new(HashMap::new()),
        }
    }
}

impl BasePort for PrebuiltBase {
    fn name(&self) -> &str {
        &self.label
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
            id: format!("{}-prebuilt", task.id),
        };
        // Write exactly the supplied bytes; OpenFab will hash + sign these.
        let changed = llm_backend::write_manifest(&task.workdir, &self.manifest)?;
        let result = RunResult {
            task_id: task.id.clone(),
            changed_files: changed,
            model: self.model.clone(),
            prompt: format!("(imported pre-built artifact from {})", self.label),
            log: format!(
                "imported {} pre-built file(s) from {} for spec '{}'",
                self.manifest.files.len(),
                self.label,
                task.spec_id
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

    fn post(&self, _channel: &str, _msg: &str) -> Result<()> {
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
    use crate::core::spec::TaskCard;

    #[test]
    fn test_prebuilt_dispatch_writes_provided_files() {
        let tmp = std::env::temp_dir().join(format!("of-prebuilt-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut files = BTreeMap::new();
        files.insert(
            "src/lib.rs".to_string(),
            "pub fn add(a:i64,b:i64)->i64{a+b}\n".to_string(),
        );
        let base = PrebuiltBase::new("agent-chat", "kimi-k2.5", files, Policy::default());
        let task = TaskCard {
            id: "t1".into(),
            spec_id: "demo".into(),
            spec_version: 1,
            intent: "add".into(),
            context: vec![],
            assumptions: vec![],
            acceptance: vec![],
            target_dir: ".".into(),
            language: None,
            workdir: tmp.clone(),
        };
        let h = base.dispatch(&task).unwrap();
        let r = base.result(&h).unwrap();
        // the provided bytes were written and reported as changed files with a hash
        assert_eq!(r.changed_files.len(), 1);
        assert_eq!(r.changed_files[0].path, "src/lib.rs");
        assert!(!r.changed_files[0].sha256.is_empty());
        assert_eq!(r.model, "kimi-k2.5");
        assert!(r.success);
        assert!(tmp.join("src/lib.rs").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
