//! Shared LLM backend used by the agent bases to turn a task into a file manifest.
//!
//! Two providers, selected by `OPENFAB_LLM` (default `claude`):
//!   • `claude`    — the local `claude` CLI (native to the claude base).
//!   • `dashscope` — Qwen via the DashScope OpenAI-compatible API (needs
//!                   `DASHSCOPE_API_KEY`), reached by shelling to `curl` (dependency
//!                   budget: no HTTP-client crate).
//!
//! The agent never touches the filesystem directly — it returns a JSON `{files:{…}}`
//! manifest that the adapter writes. That keeps the prompt hashable into provenance and
//! keeps generated code off the host until the policy-gated sandbox runs it.

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::core::sha256_hex;
use crate::core::spec::TaskCard;
use crate::ports::base::ChangedFile;

/// The agent's reply: a map of repo-relative path → file contents.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub notes: String,
}

/// Result of one generation: the files, the model id, and which provider ran it.
pub struct GenOutput {
    pub manifest: Manifest,
    pub model: String,
    pub provider: String,
}

/// Build the coding prompt from a task card (the spec's NL intent + the contract).
pub fn build_prompt(task: &TaskCard) -> String {
    let lang = task.language.as_deref().unwrap_or("any suitable language");
    let assumptions = if task.assumptions.is_empty() {
        "(none)".to_string()
    } else {
        task.assumptions.join("; ")
    };
    let context = if task.context.is_empty() {
        "(none)".to_string()
    } else {
        task.context.join(", ")
    };
    // Path guidance differs for a root-layout project (target_dir ".") vs a nested app dir.
    let (path_rule, path_example) = if task.target_dir == "." {
        (
            "Paths are relative to the repo ROOT (e.g. \"src/main.rs\", \"Cargo.toml\", \"tests/cli.rs\").".to_string(),
            "<relpath>".to_string(),
        )
    } else {
        (
            format!("Paths must start with \"{}/\".", task.target_dir),
            format!("{}/<relpath>", task.target_dir),
        )
    };

    // agent-spec mode: the acceptance criteria are BDD scenarios bound to named tests
    // (`agent-spec test: <pkg>::<filter>`), executed by `agent-spec lifecycle` (cargo test,
    // pytest, …). The agent must write BOTH the implementation AND those exact tests.
    let agent_spec_mode = task
        .acceptance
        .iter()
        .any(|a| a.check.starts_with("agent-spec test:"));

    let (checks_heading, checks, extra_rules) = if agent_spec_mode {
        let lines = task
            .acceptance
            .iter()
            .map(|a| {
                let sel = a.check.trim_start_matches("agent-spec test:").trim();
                let (_pkg, filter) = sel.split_once("::").unwrap_or(("", sel));
                format!(
                    "  - scenario \"{}\" → write a test named `{}`",
                    a.id, filter
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        (
            "BOUND TEST SCENARIOS — implement the code AND write each named test so it passes",
            lines,
            "- These tests are executed by `agent-spec lifecycle` (e.g. `cargo test`, `pytest`).\n\
             - Create each test with EXACTLY the given name so the contract's selector matches.\n\
             - Emit a complete, buildable project at the repo root (e.g. Cargo.toml + src/ + tests/).",
        )
    } else {
        let lines = task
            .acceptance
            .iter()
            .map(|a| format!("  - [{}] `{}` (must exit 0)", a.id, a.check))
            .collect::<Vec<_>>()
            .join("\n");
        (
            "MACHINE ACCEPTANCE CHECKS (your code MUST make every one pass)",
            lines,
            "- Include every file needed to pass the acceptance checks.",
        )
    };

    format!(
        r#"You are a coding agent inside OpenFab, a software fab. Implement the task below.

SPEC REF: {spec_ref}

NATURAL-LANGUAGE INTENT:
{intent}

LANGUAGE: {lang}
CONTEXT: {context}
RECORDED ASSUMPTIONS / DECISIONS / BOUNDARIES: {assumptions}

{checks_heading}:
{checks}

OUTPUT CONTRACT — respond with ONLY a single JSON object, no prose, no markdown
fences, exactly this shape:
{{"files": {{"{path_example}": "<full file contents>", ...}}, "notes": "<one line>"}}

Rules:
{extra_rules}
- Use only the standard library; assume nothing is pip/npm-installed unless declared.
- {path_rule}
- If this is a web app/server, bind to 127.0.0.1 and read the port from the PORT
  environment variable (default 8000) so it can be launched and opened in a browser.
- The JSON must be valid and parseable. Do not wrap it in code fences."#,
        spec_ref = task.spec_ref(),
        intent = task.intent,
        lang = lang,
        context = context,
        assumptions = assumptions,
    )
}

/// Run the claude CLI and return its raw assistant text + the model label.
fn claude_text(prompt: &str) -> Result<(String, String)> {
    let bin = std::env::var("OPENFAB_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string());
    let model_override = std::env::var("OPENFAB_CLAUDE_MODEL").ok();
    let mut args = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--output-format".to_string(),
        "json".to_string(),
    ];
    // Treat the claude CLI as a plain completion backend: load only project/local setting
    // sources, NOT the user's global ~/.claude (hooks, CLAUDE.md, skills, MCP). Otherwise a
    // global Stop/SessionStart hook (e.g. a knowledge-build "dream" pass) hijacks the call
    // and returns governance prose instead of the requested JSON/spec. OAuth login + model
    // still apply. Override with OPENFAB_CLAUDE_SETTING_SOURCES (empty = don't pass the flag).
    let sources = std::env::var("OPENFAB_CLAUDE_SETTING_SOURCES")
        .unwrap_or_else(|_| "project,local".to_string());
    if !sources.is_empty() {
        args.push("--setting-sources".to_string());
        args.push(sources);
    }
    if let Some(m) = &model_override {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    let stdout = run_capture(&bin, &args, timeout_secs())
        .with_context(|| format!("invoking '{bin}' — is the claude CLI installed?"))?;
    let env: ClaudeEnvelope =
        serde_json::from_str(&stdout).context("claude --output-format json was not parseable")?;
    if env.is_error {
        bail!("claude reported an error result: {}", env.result);
    }
    Ok((
        env.result,
        model_override.unwrap_or_else(|| "claude-code-cli".to_string()),
    ))
}

/// Generate with the claude CLI (the claude base's native path).
pub fn generate_claude(prompt: &str) -> Result<GenOutput> {
    let (text, model) = claude_text(prompt)?;
    Ok(GenOutput {
        manifest: parse_manifest(&text)?,
        model,
        provider: "claude-cli".to_string(),
    })
}

/// One LLM completion → raw text, respecting OPENFAB_LLM (claude default, or dashscope).
/// Returns (text, model, provider). Used for spec authoring (not file generation).
pub fn complete(prompt: &str) -> Result<(String, String, String)> {
    match std::env::var("OPENFAB_LLM").unwrap_or_default().as_str() {
        "dashscope" | "qwen" => {
            let (t, m) = dashscope_text(prompt)?;
            Ok((t, m, "dashscope".to_string()))
        }
        _ => {
            let (t, m) = claude_text(prompt)?;
            Ok((t, m, "claude-cli".to_string()))
        }
    }
}

/// Generate via the env-selected bridge backend (used by framework bases when their
/// native runtime isn't connected). Honest: the caller labels the run "bridged".
pub fn generate_bridge(prompt: &str) -> Result<GenOutput> {
    match std::env::var("OPENFAB_LLM").unwrap_or_default().as_str() {
        "dashscope" | "qwen" => generate_dashscope(prompt),
        _ => generate_claude(prompt),
    }
}

/// Call Qwen via the DashScope OpenAI-compatible API and return raw text + model.
fn dashscope_text(prompt: &str) -> Result<(String, String)> {
    let key = std::env::var("DASHSCOPE_API_KEY")
        .map_err(|_| anyhow::anyhow!("OPENFAB_LLM=dashscope but DASHSCOPE_API_KEY is not set"))?;
    let model =
        std::env::var("OPENFAB_DASHSCOPE_MODEL").unwrap_or_else(|_| "qwen-plus".to_string());
    let url = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0
    })
    .to_string();
    let args = vec![
        "-sS".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        url.to_string(),
        "-H".to_string(),
        format!("Authorization: Bearer {key}"),
        "-H".to_string(),
        "Content-Type: application/json".to_string(),
        "-d".to_string(),
        body,
    ];
    let stdout =
        run_capture("curl", &args, timeout_secs()).context("calling DashScope via curl")?;
    let env: OpenAiEnvelope = serde_json::from_str(&stdout)
        .with_context(|| format!("DashScope reply was not JSON:\n{stdout}"))?;
    let content = env
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .context("DashScope returned no choices")?;
    Ok((content, model))
}

/// Generate with Qwen via DashScope.
pub fn generate_dashscope(prompt: &str) -> Result<GenOutput> {
    let (text, model) = dashscope_text(prompt)?;
    Ok(GenOutput {
        manifest: parse_manifest(&text)?,
        model,
        provider: "dashscope".to_string(),
    })
}

/// A spec drafted by the LLM from a natural-language intent. The acceptance criteria —
/// the machine-checkable definition of "done" — come from the model, not the human.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthoredSpec {
    pub language: String,
    #[serde(default = "default_app_dir")]
    pub target_dir: String,
    pub acceptance: Vec<AuthoredCheck>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthoredCheck {
    pub id: String,
    pub check: String,
}

fn default_app_dir() -> String {
    "app".to_string()
}

/// Ask the LLM to author a build spec (incl. acceptance criteria) from an NL intent.
/// Returns (spec, model, provider).
pub fn author_spec(intent: &str) -> Result<(AuthoredSpec, String, String)> {
    let prompt = format!(
        r#"You are OpenFab's SPEC AUTHOR. Turn the user's natural-language request into a
machine-checkable build spec. Respond with ONLY a JSON object (no prose, no code fences):

{{"language":"<python|node|rust|...>",
  "target_dir":"app",
  "acceptance":[{{"id":"a1-<slug>","check":"<shell command; exit 0 = pass>"}}, ...],
  "assumptions":["<assumption you made about an ambiguous part>", ...],
  "open_questions":["<question you'd ask the human>", ...]}}

Rules for `acceptance` (this is the contract the built software is verified against):
- 2 to 4 checks that GENUINELY verify the request. exit 0 = pass. Available: python3,
  node, cargo, bash, sh, grep, test. No network (localhost only). Standard library only.
- ROBUSTNESS — a check must NEVER fail because of its own syntax. Prefer
  `python3 -c "...; assert ..."` or `grep -F 'literal'` (fixed string). Avoid fragile
  regex; NEVER write a bracket range that's invalid, e.g. `[+\-*/]` errors in grep.
- Each check must finish quickly. NEVER start a web server, GUI, or any long-running /
  blocking process in a check — it hangs the sandbox.
- CLI app: run it with arguments and assert its output (it must exit on its own).
- Web app OR server: verify it STRUCTURALLY only — the entry file exists; a server reads
  the PORT env var; the key routes/handlers/functions/elements are present (use `grep -F`
  fixed strings). Do NOT launch it. The human verifies the *running* app in a browser via
  the "Run the app" button and approves — that is the behavioural check for UIs/servers.
- Paths are relative to the repo root and live under target_dir (e.g. "app/...").

USER REQUEST:
{intent}"#,
        intent = intent
    );
    let (text, model, provider) = complete(&prompt)?;
    let spec = parse_authored_spec(&text)?;
    Ok((spec, model, provider))
}

fn parse_authored_spec(text: &str) -> Result<AuthoredSpec> {
    let t = strip_fences(text.trim());
    if let Ok(s) = serde_json::from_str::<AuthoredSpec>(&t) {
        return Ok(s);
    }
    if let (Some(i), Some(j)) = (t.find('{'), t.rfind('}')) {
        if j > i {
            if let Ok(s) = serde_json::from_str::<AuthoredSpec>(&t[i..=j]) {
                return Ok(s);
            }
        }
    }
    bail!("could not parse an authored spec from the model reply:\n{text}")
}

/// Write a manifest into `workdir`, returning the changed-file records (with content
/// hashes for attribution). Refuses path escapes outside the workdir.
pub fn write_manifest(workdir: &std::path::Path, manifest: &Manifest) -> Result<Vec<ChangedFile>> {
    if manifest.files.is_empty() {
        bail!("agent returned an empty file manifest");
    }
    let mut changed = Vec::new();
    for (rel, contents) in &manifest.files {
        let safe_rel = rel.trim_start_matches('/');
        if safe_rel.contains("..") {
            bail!("agent tried to write outside the workdir: {rel}");
        }
        let abs = workdir.join(safe_rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs, contents)
            .with_context(|| format!("writing generated file {}", abs.display()))?;
        changed.push(ChangedFile {
            path: safe_rel.to_string(),
            lines: contents.lines().count(),
            sha256: sha256_hex(contents.as_bytes()),
        });
    }
    Ok(changed)
}

fn timeout_secs() -> Duration {
    std::env::var("OPENFAB_CLAUDE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(300))
}

#[derive(Deserialize)]
struct ClaudeEnvelope {
    #[serde(default)]
    result: String,
    #[serde(default)]
    is_error: bool,
}

#[derive(Deserialize)]
struct OpenAiEnvelope {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
}
#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}
#[derive(Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: String,
}

/// Extract the file manifest from an agent reply; tolerant of prose / code fences.
pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let text = text.trim();
    if let Ok(m) = serde_json::from_str::<Manifest>(text) {
        return Ok(m);
    }
    let cleaned = strip_fences(text);
    if let Ok(m) = serde_json::from_str::<Manifest>(&cleaned) {
        return Ok(m);
    }
    if let (Some(i), Some(j)) = (cleaned.find('{'), cleaned.rfind('}')) {
        if j > i {
            if let Ok(m) = serde_json::from_str::<Manifest>(&cleaned[i..=j]) {
                return Ok(m);
            }
        }
    }
    bail!("could not extract a {{files:...}} manifest from the agent reply:\n{text}")
}

fn strip_fences(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|x| x.1).unwrap_or("");
        return rest.trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}

/// Spawn a command, capture stdout, hard-kill after `timeout`. Drains pipes on threads
/// to avoid buffer deadlock.
fn run_capture(bin: &str, args: &[String], timeout: Duration) -> Result<String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {bin}"))?;
    let pid = child.id();
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        let _ = tx_out.send(s);
    });
    thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        let _ = tx_err.send(s);
    });
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait().context("waiting on subprocess")? {
            Some(status) => {
                let stdout = rx_out.recv().unwrap_or_default();
                let stderr = rx_err.recv().unwrap_or_default();
                if !status.success() {
                    bail!(
                        "{bin} exited with {:?}\nstderr:\n{}",
                        status.code(),
                        stderr.trim()
                    );
                }
                return Ok(stdout);
            }
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
                    bail!("{bin} timed out after {:?}", timeout);
                }
                thread::sleep(Duration::from_millis(150));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_manifest() {
        let m = parse_manifest(r#"{"files": {"app/x.py": "print(1)\n"}, "notes": "ok"}"#).unwrap();
        assert_eq!(m.files["app/x.py"], "print(1)\n");
    }

    #[test]
    fn parses_fenced_manifest() {
        let m =
            parse_manifest("```json\n{\"files\": {\"a\": \"b\"}, \"notes\": \"\"}\n```").unwrap();
        assert_eq!(m.files.len(), 1);
    }

    #[test]
    fn parses_with_surrounding_prose() {
        let m = parse_manifest("Here:\n{\"files\": {\"a\": \"b\"}}\nDone.").unwrap();
        assert_eq!(m.files["a"], "b");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_manifest("not json at all").is_err());
    }
}
