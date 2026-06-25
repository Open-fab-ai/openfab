//! Shared LLM backend used by the agent bases to turn a task into a file manifest.
//!
//! Providers, selected by `OPENFAB_LLM` (default `claude`):
//!   • `claude`    — the local `claude` CLI (native to the claude base).
//!   • `dashscope` — Qwen via the DashScope OpenAI-compatible API (needs
//!                   `DASHSCOPE_API_KEY`), reached by shelling to `curl` (dependency
//!                   budget: no HTTP-client crate).
//!   • `ollama`    — a LOCAL model served by Ollama's OpenAI-compatible API (no API key,
//!                   no network). `OPENFAB_OLLAMA_URL` (default http://localhost:11434),
//!                   `OPENFAB_OLLAMA_MODEL` (default llama3.1). Same `curl` path as above.
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
    let checks = task
        .acceptance
        .iter()
        .map(|a| format!("  - [{}] `{}` (must exit 0)", a.id, a.check))
        .collect::<Vec<_>>()
        .join("\n");
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
    format!(
        r#"You are a coding agent inside OpenFab, a software fab. Implement the task below.

SPEC REF: {spec_ref}

NATURAL-LANGUAGE INTENT:
{intent}

LANGUAGE: {lang}
TARGET DIRECTORY (all files go under this, relative paths): {target_dir}/
CONTEXT: {context}
RECORDED ASSUMPTIONS: {assumptions}

MACHINE ACCEPTANCE CHECKS (your code MUST make every one pass):
{checks}

OUTPUT CONTRACT — respond with ONLY a single JSON object, no prose, no markdown
fences, exactly this shape:
{{"files": {{"{target_dir}/<relpath>": "<full file contents>", ...}}, "notes": "<one line>"}}

Rules:
- Include every file needed to pass the acceptance checks.
- SELF-CONSISTENCY: include EVERY file your own code references. If your server reads
  `index.html` (or any static asset), you MUST also emit that file — never ship a server
  that loads a file you didn't create.
- Use only the standard library; assume nothing is pip/npm-installed.
- Paths must start with "{target_dir}/".
- If this is a web app/server, bind to 127.0.0.1 and read the port from the PORT
  environment variable (default 8000) so it can be launched and opened in a browser.
- The JSON must be valid and parseable. Do not wrap it in code fences."#,
        spec_ref = task.spec_ref(),
        intent = task.intent,
        lang = lang,
        target_dir = task.target_dir,
        context = context,
        assumptions = assumptions,
        checks = checks,
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

/// Run the codex CLI non-interactively (`codex exec`) and return its final message + model.
/// Uses `--output-last-message <file>` so we capture ONLY the agent's final reply (the JSON
/// manifest), not the event log. `OPENFAB_CODEX_BIN` overrides the binary, `OPENFAB_CODEX_MODEL`
/// the model. Faster than the claude CLI in practice.
fn codex_text(prompt: &str) -> Result<(String, String)> {
    let bin = std::env::var("OPENFAB_CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
    let model_override = std::env::var("OPENFAB_CODEX_MODEL")
        .ok()
        .filter(|s| !s.is_empty());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out_path =
        std::env::temp_dir().join(format!("openfab-codex-{}-{nanos}.txt", std::process::id()));
    let mut args = vec![
        "exec".to_string(),
        "--skip-git-repo-check".to_string(),
        "--dangerously-bypass-approvals-and-sandbox".to_string(),
        "-o".to_string(),
        out_path.display().to_string(),
    ];
    if let Some(m) = &model_override {
        args.push("-m".to_string());
        args.push(m.clone());
    }
    args.push(prompt.to_string());
    run_capture(&bin, &args, timeout_secs()).with_context(|| {
        format!("invoking '{bin} exec' — is the codex CLI installed + logged in?")
    })?;
    let text = std::fs::read_to_string(&out_path)
        .context("reading codex --output-last-message file (no final message produced?)")?;
    let _ = std::fs::remove_file(&out_path);
    Ok((
        text,
        model_override.unwrap_or_else(|| "codex-cli".to_string()),
    ))
}

/// Generate with the codex CLI (the codex base's native path).
pub fn generate_codex(prompt: &str) -> Result<GenOutput> {
    let (text, model) = codex_text(prompt)?;
    Ok(GenOutput {
        manifest: parse_manifest(&text)?,
        model,
        provider: "codex-cli".to_string(),
    })
}

/// One LLM completion → raw text, respecting OPENFAB_LLM (claude default, or dashscope).
/// Returns (text, model, provider). Used for spec authoring (not file generation).
pub fn complete(prompt: &str) -> Result<(String, String, String)> {
    complete_with(prompt, None)
}

/// Like `complete`, but with an optional per-call model override (else the env default).
/// The override only applies to the OpenAI-compatible providers (ollama/dashscope); the
/// claude CLI keeps its own `OPENFAB_CLAUDE_MODEL`.
pub fn complete_with(prompt: &str, model: Option<&str>) -> Result<(String, String, String)> {
    match std::env::var("OPENFAB_LLM").unwrap_or_default().as_str() {
        "dashscope" | "qwen" => {
            let (t, m) = dashscope_text(prompt, model)?;
            Ok((t, m, "dashscope".to_string()))
        }
        "ollama" => {
            let (t, m) = ollama_text(prompt, model)?;
            Ok((t, m, "ollama".to_string()))
        }
        _ => {
            let (t, m) = claude_text(prompt)?;
            Ok((t, m, "claude-cli".to_string()))
        }
    }
}

/// Generate via the env-selected bridge backend (used by framework bases when their
/// native runtime isn't connected). Honest: the caller labels the run "bridged".
/// `model` is an optional per-run override for the ollama/dashscope providers.
pub fn generate_bridge(prompt: &str, model: Option<&str>) -> Result<GenOutput> {
    match std::env::var("OPENFAB_LLM").unwrap_or_default().as_str() {
        "dashscope" | "qwen" => generate_dashscope(prompt, model),
        "ollama" => generate_ollama(prompt, model),
        _ => generate_claude(prompt),
    }
}

/// Resolve the Ollama model: explicit per-run override → `OPENFAB_OLLAMA_MODEL` → default.
fn ollama_model(override_: Option<&str>) -> String {
    override_
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("OPENFAB_OLLAMA_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "llama3.1".to_string())
}

/// List the models available on the configured Ollama endpoint (OpenAI-compatible
/// `/v1/models`). Uses `OPENFAB_OLLAMA_KEY` if set (cloud). Returns ids, sorted. The key
/// stays server-side — this is what the UI's model picker is populated from.
pub fn list_ollama_models() -> Result<Vec<String>> {
    let base = std::env::var("OPENFAB_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let url = format!("{}/v1/models", base.trim_end_matches('/'));
    let mut args = vec!["-sS".to_string(), url.clone()];
    if let Ok(key) = std::env::var("OPENFAB_OLLAMA_KEY") {
        if !key.is_empty() {
            args.push("-H".to_string());
            args.push(format!("Authorization: Bearer {key}"));
        }
    }
    let stdout = run_capture("curl", &args, timeout_secs())
        .with_context(|| format!("listing models from {url}"))?;
    #[derive(Deserialize)]
    struct ModelList {
        data: Vec<ModelId>,
    }
    #[derive(Deserialize)]
    struct ModelId {
        id: String,
    }
    let list: ModelList = serde_json::from_str(&stdout)
        .with_context(|| format!("model list was not JSON:\n{stdout}"))?;
    let mut ids: Vec<String> = list.data.into_iter().map(|m| m.id).collect();
    ids.sort();
    Ok(ids)
}

/// Call Qwen via the DashScope OpenAI-compatible API and return raw text + model.
fn dashscope_text(prompt: &str, model_override: Option<&str>) -> Result<(String, String)> {
    let key = std::env::var("DASHSCOPE_API_KEY")
        .map_err(|_| anyhow::anyhow!("OPENFAB_LLM=dashscope but DASHSCOPE_API_KEY is not set"))?;
    let model = model_override
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("OPENFAB_DASHSCOPE_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "qwen-plus".to_string());
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
pub fn generate_dashscope(prompt: &str, model: Option<&str>) -> Result<GenOutput> {
    let (text, model) = dashscope_text(prompt, model)?;
    Ok(GenOutput {
        manifest: parse_manifest(&text)?,
        model,
        provider: "dashscope".to_string(),
    })
}

/// Call a LOCAL model via Ollama's OpenAI-compatible API (no API key, no network egress).
/// Endpoint `OPENFAB_OLLAMA_URL` (default http://localhost:11434), model
/// `OPENFAB_OLLAMA_MODEL` (default llama3.1). The same OpenAI envelope as DashScope, so a
/// hosted Ollama Cloud endpoint works too — point `OPENFAB_OLLAMA_URL` at it and pass a key
/// via `OPENFAB_OLLAMA_KEY`.
fn ollama_text(prompt: &str, model_override: Option<&str>) -> Result<(String, String)> {
    let base = std::env::var("OPENFAB_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
    let model = ollama_model(model_override);
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0,
        "stream": false,
        "response_format": {"type": "json_object"}
    })
    .to_string();
    let mut args = vec![
        "-sS".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        url.clone(),
        "-H".to_string(),
        "Content-Type: application/json".to_string(),
    ];
    // Optional bearer key (only needed for a hosted/cloud Ollama endpoint).
    if let Ok(key) = std::env::var("OPENFAB_OLLAMA_KEY") {
        if !key.is_empty() {
            args.push("-H".to_string());
            args.push(format!("Authorization: Bearer {key}"));
        }
    }
    args.push("-d".to_string());
    args.push(body);
    let stdout = run_capture("curl", &args, timeout_secs())
        .with_context(|| format!("calling Ollama at {url} — is `ollama serve` running?"))?;
    let env: OpenAiEnvelope = serde_json::from_str(&stdout)
        .with_context(|| format!("Ollama reply was not JSON:\n{stdout}"))?;
    let content = env
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .context("Ollama returned no choices (model not pulled? try `ollama pull <model>`)")?;
    Ok((content, model))
}

/// Generate a file manifest with an Ollama model (local or cloud).
pub fn generate_ollama(prompt: &str, model: Option<&str>) -> Result<GenOutput> {
    let (text, model) = ollama_text(prompt, model)?;
    Ok(GenOutput {
        manifest: parse_manifest(&text)?,
        model,
        provider: "ollama".to_string(),
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
pub fn author_spec(intent: &str, model: Option<&str>) -> Result<(AuthoredSpec, String, String)> {
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
    let (text, model, provider) = complete_with(&prompt, model)?;
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
