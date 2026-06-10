//! `BasePort` — the key abstraction (PRD §3). The agent runtime/collaboration "base"
//! is swappable (AgentScope, HiClaw, agent-chat, OpenHands, or a local `claude` CLI)
//! behind this one trait. Capability-negotiated: OpenFab fills gaps when a base lacks
//! a capability (e.g. provides its own sandbox if the base has none).

use std::path::Path;

use anyhow::Result;

use crate::core::spec::TaskCard;

/// What a base can do for us. OpenFab inspects this and fills any gaps itself.
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub orchestrate: bool,
    pub comms: bool,
    pub memory: bool,
    pub sandbox: bool,
}

/// Opaque handle to a dispatched run (the base may execute async; v0.1 bases execute
/// synchronously inside `dispatch` and stash the result for `result`).
#[derive(Debug, Clone)]
pub struct RunHandle {
    pub id: String,
}

/// One file the agent created or changed, with the evidence needed for attribution.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    /// Line count authored (drives the `file/line ranges` in the generation predicate).
    pub lines: usize,
    pub sha256: String,
}

/// The outcome of an agent run — the raw material for the provenance predicate.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub task_id: String,
    pub changed_files: Vec<ChangedFile>,
    /// Model identifier the base actually used (recorded in provenance).
    pub model: String,
    /// The exact prompt sent to the agent (hashed into provenance, not stored raw).
    pub prompt: String,
    /// Human-readable transcript/summary — goes into the decision log.
    pub log: String,
    pub success: bool,
}

/// Result of running a command in the sandbox.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecResult {
    pub fn passed(&self) -> bool {
        self.exit_code == 0
    }
}

/// The swappable base. Mirrors PRD §3 (`dispatch`/`result`/`post`/`memory`/`sandbox`).
/// Note: PRD's `events()` live human-feedback stream (Matrix when base=HiClaw) is a
/// v0.1 simplification — feedback enters the loop as an explicit input instead; the
/// `events` seam is documented for the HiClaw/Matrix adapter (PRD §3).
pub trait BasePort {
    /// Stable name for logs/provenance (e.g. "claude-cli", "mock", "hiclaw").
    fn name(&self) -> &str;

    /// How this base actually executed the task, recorded honestly in provenance (R14):
    /// "native" (the base's own runtime ran it), "bridged" (native runtime not detected,
    /// so OpenFab's LLM backend ran it under this base's identity), or "deterministic".
    fn runtime_mode(&self) -> &str {
        "native"
    }

    fn capabilities(&self) -> Capabilities;

    // --- orchestration ---
    fn dispatch(&self, task: &TaskCard) -> Result<RunHandle>;
    fn result(&self, h: &RunHandle) -> Result<RunResult>;

    // --- comms (human-in-the-loop; Matrix room timeline when base = HiClaw) ---
    fn post(&self, channel: &str, msg: &str) -> Result<()>;

    // --- memory (AgentScope ReMe when base = AgentScope) ---
    fn memory_get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn memory_put(&self, key: &str, val: &[u8]) -> Result<()>;

    // --- sandbox (OpenFab provides its own if the base lacks one) ---
    fn run_sandboxed(&self, cmd: &[String], workdir: &Path) -> Result<ExecResult>;
}
