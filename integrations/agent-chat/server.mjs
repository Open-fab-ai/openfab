#!/usr/bin/env node
// OpenFab <-> agent-chat native base adapter.
//
// WHAT THIS IS (read the honesty note in the OpenFab StructuredOutput too):
//   agent-chat (~/projects/agent-chat by default) is NOT itself a coding
//   agent. It is a local-first ORCHESTRATION layer for tmux-based CLI agents
//   (Claude Code / Codex): it provisions agent homes, launches them into a
//   project workspace, and steers them via messages, MCP tools, and a push
//   relay. The ONLY LLM-calling code inside agent-chat is its "subconscious /
//   supervisor" path — backend-v2.js `callSubconsciousRuntimeLlm()` — an
//   OpenAI-compatible POST to `<endpoint>/chat/completions` with body
//   { model, temperature, max_tokens, messages:[system,user] }, Bearer auth,
//   reading `choices[0].message.content`. That client is NOT exported, so this
//   adapter REPLICATES that exact wire contract (same body shape, same field
//   extraction) pointed at local Ollama's OpenAI-compatible /v1 endpoint.
//
// This server speaks the OpenFab NATIVE BASE dispatch contract
// (src/adapters/base_framework.rs :: dispatch_native):
//   POST OPENFAB_AGENTCHAT_URL  with JSON
//     { intent, target_dir, language, acceptance:[<shell check strings>] }
//   ->  { files: { "<target_dir>/<relpath>": "<contents>", ... }, notes }
//
// TWO MODES (AGENTCHAT_NATIVE_MODE):
//   "llm"        (default) — drive agent-chat's own LLM client wire-format
//                against Ollama to synthesize the file manifest. Robust,
//                synchronous, no external CLI needed. This is the path that
//                will actually run on this box today. It exercises agent-chat's
//                LLM-client contract, NOT its multi-agent tmux coordination.
//   "orchestrate" — the GENUINE agent-chat loop: provision + launch a real
//                tmux CLI agent (claude/codex) into target_dir via
//                `bin/agentchat up-v1`, send it the OpenFab intent with
//                `agentchat send`, wait for it to write files on disk, harvest
//                them. This is agent-chat's real capability, but it needs a CLI
//                agent (claude present; codex absent here) configured to talk
//                to Ollama — which agent-chat itself does not configure. Gated
//                behind the env flag for that reason. Falls back to "llm" if
//                prerequisites are missing (and says so in `notes`).
//
// No vacuous success (global rule R14): an empty/partial manifest is a 502, not
// a clean pass. Empty Ollama output = failure.

import http from 'node:http';
import { spawnSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import path from 'node:path';

// ----- config (all overridable by env) -----------------------------------
const PORT = Number(process.env.AGENTCHAT_ADAPTER_PORT || 8741);
const HOST = process.env.AGENTCHAT_ADAPTER_HOST || '127.0.0.1';
const MODE = (process.env.AGENTCHAT_NATIVE_MODE || 'llm').toLowerCase();

// agent-chat's LLM client speaks the OpenAI-compatible /chat/completions wire
// format. Ollama exposes exactly that at /v1/chat/completions.
const LLM_ENDPOINT =
  process.env.AGENTCHAT_LLM_ENDPOINT ||
  'http://localhost:11434/v1/chat/completions';
const LLM_MODEL = process.env.AGENTCHAT_LLM_MODEL || 'qwen3:8b';
const LLM_API_KEY = process.env.AGENTCHAT_LLM_KEY || 'ollama'; // Ollama ignores it
const LLM_TIMEOUT_MS = Number(process.env.AGENTCHAT_LLM_TIMEOUT_MS || 180000);
const LLM_MAX_TOKENS = Number(process.env.AGENTCHAT_LLM_MAX_TOKENS || 8192);
const LLM_TEMPERATURE = Number(process.env.AGENTCHAT_LLM_TEMPERATURE ?? 0.1);

// orchestrate mode wiring
const AGENTCHAT_REPO =
  process.env.AGENTCHAT_REPO ||
  path.join(process.env.HOME || '', 'projects', 'agent-chat');
const AGENTCHAT_BIN = path.join(AGENTCHAT_REPO, 'bin', 'agentchat');
const ORCH_AGENT_TYPE = process.env.AGENTCHAT_AGENT_TYPE || 'claude';
const ORCH_AGENT_NAME = process.env.AGENTCHAT_AGENT_NAME || 'openfab-builder';
const ORCH_WAIT_MS = Number(process.env.AGENTCHAT_ORCH_WAIT_MS || 240000);
const ORCH_POLL_MS = Number(process.env.AGENTCHAT_ORCH_POLL_MS || 4000);

// ----- the prompt agent-chat's LLM client receives -----------------------
// Mirrors OpenFab's own build_prompt() output contract so the manifest the Rust
// side parses (parse_manifest) drops in cleanly: paths under target_dir/, valid
// JSON, no fences required (parser tolerates them anyway).
function buildPrompt({ intent, target_dir, language, acceptance }) {
  const lang = language || 'any suitable language';
  const checks =
    Array.isArray(acceptance) && acceptance.length
      ? acceptance.map((c) => `  - \`${c}\` (must exit 0)`).join('\n')
      : '  (none)';
  return `You are a coding agent. Implement the task below in full.

NATURAL-LANGUAGE INTENT:
${intent}

LANGUAGE: ${lang}
TARGET DIRECTORY (all files go under this, relative paths): ${target_dir}/

MACHINE ACCEPTANCE CHECKS (your code MUST make every one pass):
${checks}

OUTPUT CONTRACT — respond with ONLY a single JSON object, no prose, no markdown
fences, exactly this shape:
{"files": {"${target_dir}/<relpath>": "<full file contents>", ...}, "notes": "<one line>"}

Rules:
- Include every file needed to pass the acceptance checks.
- Use only the standard library; assume nothing is pip/npm-installed.
- Every path MUST start with "${target_dir}/". No "..", no absolute paths.
- If this is a web app/server, bind to 127.0.0.1 and read the port from the PORT
  environment variable (default 8000).
- The JSON must be valid and parseable. Do not wrap it in code fences.`;
}

// ----- agent-chat's real LLM client wire format (replicated) -------------
// Faithful to backend-v2.js callSubconsciousRuntimeLlm(): same body shape,
// same Bearer header, same choices[0].message.content extraction.
async function callAgentChatLlm(prompt, model) {
  const body = {
    model: model || LLM_MODEL,
    temperature: LLM_TEMPERATURE,
    max_tokens: LLM_MAX_TOKENS,
    messages: [
      {
        role: 'system',
        content:
          'You are a strict JSON generator. Output only one valid JSON object and nothing else.',
      },
      { role: 'user', content: prompt },
    ],
  };
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), LLM_TIMEOUT_MS);
  try {
    const resp = await fetch(LLM_ENDPOINT, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${LLM_API_KEY}`,
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    if (!resp.ok) {
      const errText = await resp.text().catch(() => '');
      throw new Error(`llm http ${resp.status}: ${errText.slice(0, 300)}`);
    }
    const json = await resp.json();
    const content = json?.choices?.[0]?.message?.content;
    if (!content || !content.trim()) {
      // R14: empty completion is a failure, never a vacuous pass.
      throw new Error('llm response missing choices[0].message.content');
    }
    return content;
  } finally {
    clearTimeout(timer);
  }
}

// ----- manifest extraction (tolerant, mirrors OpenFab parse_manifest) ----
// Some local models think aloud; strip <think> blocks and code fences, then
// take the outermost {...}. Throws on no manifest (no silent swallow, R5).
function extractManifest(text) {
  let t = String(text).trim();
  t = t.replace(/<think>[\s\S]*?<\/think>/gi, '').trim();
  const tryParse = (s) => {
    try {
      const m = JSON.parse(s);
      if (m && typeof m === 'object' && m.files && typeof m.files === 'object') {
        return m;
      }
    } catch {
      /* fall through to next strategy */
    }
    return null;
  };
  let m = tryParse(t);
  if (m) return m;
  if (t.startsWith('```')) {
    const inner = t.replace(/^```[a-zA-Z]*\n?/, '').replace(/```\s*$/, '');
    m = tryParse(inner.trim());
    if (m) return m;
  }
  const i = t.indexOf('{');
  const j = t.lastIndexOf('}');
  if (i !== -1 && j > i) {
    m = tryParse(t.slice(i, j + 1));
    if (m) return m;
  }
  throw new Error(
    `could not extract a {files:...} manifest from the model reply:\n${t.slice(0, 600)}`,
  );
}

// Enforce the dispatch contract on the files map. Empty => failure (R14).
function validateFiles(files, target_dir) {
  const keys = Object.keys(files || {});
  if (keys.length === 0) {
    throw new Error('manifest contained zero files (vacuous result rejected)');
  }
  const prefix = `${target_dir}/`;
  const fixed = {};
  for (const [rawKey, val] of Object.entries(files)) {
    if (typeof val !== 'string') {
      throw new Error(`file "${rawKey}" has non-string contents`);
    }
    let key = rawKey.replace(/^\/+/, '');
    if (key.includes('..')) {
      throw new Error(`file path escapes workdir: ${rawKey}`);
    }
    // Force every path under target_dir/ as the contract requires.
    if (!key.startsWith(prefix)) {
      key = prefix + key.replace(new RegExp(`^${prefix}`), '');
    }
    fixed[key] = val;
  }
  return fixed;
}

// ----- mode: llm (default) -----------------------------------------------
async function dispatchLlm(task) {
  const prompt = buildPrompt(task);
  // task.model = OpenFab's per-run picker (else the adapter's AGENTCHAT_LLM_MODEL default).
  const raw = await callAgentChatLlm(prompt, task.model);
  const manifest = extractManifest(raw);
  const files = validateFiles(manifest.files, task.target_dir);
  const notes =
    (typeof manifest.notes === 'string' && manifest.notes.trim()) ||
    `agent-chat LLM client (Ollama ${LLM_MODEL}) implemented intent`;
  return {
    files,
    notes: `[agent-chat:llm-client model=${LLM_MODEL}] ${notes}`,
  };
}

// ----- mode: team-native (the swarm runs INSIDE agent-chat) --------------
// This is the architecturally-correct path: agent-chat coordinates the coder ->
// reviewer -> revise team itself (POST /api/team-build, fast in-process LLM calls
// on Ollama). The adapter is thin glue — it forwards the task and harvests {files}.
const AGENTCHAT_BACKEND_URL =
  process.env.AGENTCHAT_BACKEND_URL || 'http://127.0.0.1:8090';
async function dispatchTeamNative(task) {
  const resp = await fetch(`${AGENTCHAT_BACKEND_URL}/api/team-build`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      intent: task.intent,
      target_dir: task.target_dir,
      language: task.language,
      acceptance: task.acceptance || [],
      model: task.model,
    }),
  });
  if (!resp.ok) {
    throw new Error(`agent-chat /api/team-build returned HTTP ${resp.status}: ${(await resp.text().catch(() => '')).slice(0, 200)}`);
  }
  const out = await resp.json();
  if (!out || !out.files) throw new Error('agent-chat team-build returned no files');
  const files = validateFiles(out.files, task.target_dir);
  const reviewed = out.review && out.review.ok === false
    ? `reviewer found ${(out.review.issues || []).length} issue(s), coder revised`
    : 'reviewer passed';
  return {
    files,
    notes: `[agent-chat:team-native model=${out.model}] coder→reviewer→revise (${out.rounds} round${out.rounds === 1 ? '' : 's'}, ${reviewed})`,
  };
}

// ----- mode: orchestrate (genuine agent-chat tmux loop) ------------------
function haveCliAgent(type) {
  const r = spawnSync('which', [type], { encoding: 'utf8' });
  return r.status === 0 && r.stdout.trim().length > 0;
}

function listFilesUnder(dir) {
  const out = {};
  if (!existsSync(dir)) return out;
  const walk = (d) => {
    for (const ent of readdirSync(d)) {
      if (ent === '.git' || ent === 'node_modules') continue;
      const p = path.join(d, ent);
      const st = statSync(p);
      if (st.isDirectory()) walk(p);
      else if (st.isFile()) out[p] = st.mtimeMs;
    }
  };
  walk(dir);
  return out;
}

async function dispatchOrchestrate(task) {
  // Prereqs for the genuine loop: agentchat bin + tmux + a CLI agent.
  const tmuxOk = spawnSync('which', ['tmux'], { encoding: 'utf8' }).status === 0;
  const agentChatOk = existsSync(AGENTCHAT_BIN);
  const cliOk = haveCliAgent(ORCH_AGENT_TYPE);
  if (!tmuxOk || !agentChatOk || !cliOk) {
    const missing = [
      !tmuxOk && 'tmux',
      !agentChatOk && `agentchat bin (${AGENTCHAT_BIN})`,
      !cliOk && `${ORCH_AGENT_TYPE} CLI`,
    ]
      .filter(Boolean)
      .join(', ');
    const r = await dispatchLlm(task);
    return {
      files: r.files,
      notes: `[agent-chat:orchestrate->fell-back-to-llm missing=${missing}] ${r.notes}`,
    };
  }

  // The target_dir lives under OpenFab's workdir; OpenFab passes target_dir as a
  // repo-relative slug. For orchestrate mode we materialize into a scratch
  // project that the real agent edits, then return its files.
  const buildRoot = path.join(
    process.env.AGENTCHAT_ORCH_SCRATCH || '/tmp/openfab-agentchat-builds',
    `${ORCH_AGENT_NAME}-${Date.now()}`,
  );
  const scratch = path.join(buildRoot, task.target_dir);
  spawnSync('mkdir', ['-p', scratch]);
  const before = listFilesUnder(buildRoot);

  // 1) Provision + launch a REAL tmux CLI agent into the workspace.
  const up = spawnSync(
    AGENTCHAT_BIN,
    [
      'up-v1',
      ORCH_AGENT_NAME,
      ORCH_AGENT_TYPE,
      '--project',
      path.dirname(scratch),
      '--project-mode',
      'symlink',
      '--fresh',
    ],
    { encoding: 'utf8', cwd: AGENTCHAT_REPO, timeout: 60000 },
  );
  if (up.status !== 0) {
    throw new Error(
      `agentchat up-v1 failed (${up.status}): ${(up.stderr || up.stdout || '').slice(0, 400)}`,
    );
  }

  // Give the freshly-launched CLI agent time to boot to its prompt before we
  // inject the task (Claude Code needs ~30s to load its context).
  await new Promise((r) => setTimeout(r, Number(process.env.AGENTCHAT_ORCH_BOOT_MS || 35000)));

  // 2) Deliver the task to the REAL agent by injecting it into its tmux pane.
  //    agent-chat's `send` is agent-to-agent and must run *inside* a pane; the
  //    operator->agent path is a direct tmux injection. Keep it ONE line so
  //    Claude Code submits it as a single instruction (newlines submit early).
  const checks = (task.acceptance || []).join('  AND  ');
  const oneLine =
    `Build this and write ALL files under the absolute directory ${buildRoot}/ using absolute paths ` +
    `(for example ${buildRoot}/${task.target_dir}/<file>) — do NOT write anywhere else. The request: ${task.intent}. ` +
    `Include EVERY file your code references: if your server reads index.html (or any static file), you MUST also create that file. ` +
    (checks ? `It MUST satisfy these shell checks — cd ${buildRoot} and actually run EACH one yourself; do NOT reply DONE until every one exits 0: ${checks}. ` : '') +
    `Create the files now under ${buildRoot}/, run the checks, then reply DONE.`;
  const flat = oneLine.replace(/\s+/g, ' ').trim();
  spawnSync('tmux', ['send-keys', '-t', ORCH_AGENT_NAME, '-l', flat], { encoding: 'utf8' });
  spawnSync('tmux', ['send-keys', '-t', ORCH_AGENT_NAME, 'Enter'], { encoding: 'utf8' });

  // 3) Wait for the agent to write files on disk (poll mtimes).
  const deadline = Date.now() + ORCH_WAIT_MS;
  let after = {};
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, ORCH_POLL_MS));
    after = listFilesUnder(buildRoot);
    const newOrChanged = Object.keys(after).filter(
      (p) => before[p] === undefined || after[p] > before[p],
    );
    // Heuristic: agent is done when it has produced files and they've been
    // stable for one poll interval.
    if (newOrChanged.length > 0) {
      const settle = {};
      Object.assign(settle, after);
      await new Promise((r) => setTimeout(r, ORCH_POLL_MS));
      const again = listFilesUnder(buildRoot);
      const stable = newOrChanged.every((p) => again[p] === settle[p]);
      if (stable) {
        after = again;
        break;
      }
    }
  }

  // 4) Harvest into the dispatch manifest (keys relative to OpenFab workdir).
  const files = {};
  for (const abs of Object.keys(after)) {
    const rel = path.relative(buildRoot, abs);
    if (!rel || rel.includes('..')) continue;
    files[rel] = readFileSync(abs, 'utf8');
  }
  // R14: real agent produced nothing => failure, not a clean pass.
  validateFiles(files, task.target_dir);
  return {
    files,
    notes: `[agent-chat:orchestrate agent=${ORCH_AGENT_NAME}/${ORCH_AGENT_TYPE}] real tmux agent loop wrote ${Object.keys(files).length} file(s)`,
  };
}

// ----- mode: team (genuine multi-agent: coder + reviewer critique→revise) -
const ORCH_BOOT_MS = Number(process.env.AGENTCHAT_ORCH_BOOT_MS || 35000);
const CODER_AGENT = process.env.AGENTCHAT_CODER_NAME || 'openfab-builder';
const CODER_TYPE = process.env.AGENTCHAT_CODER_TYPE || 'claude';
const REVIEWER_AGENT = process.env.AGENTCHAT_REVIEWER_NAME || 'openfab-reviewer';
// Default reviewer = a second Claude Code agent (a persistent TUI we can drive via
// tmux injection). codex's CLI is NOT a persistent prompt — it bootstraps and returns
// to a shell, so injected tasks hit the shell, not the agent. Set
// AGENTCHAT_REVIEWER_TYPE=codex only once a codex-specific driver is wired.
const REVIEWER_TYPE = process.env.AGENTCHAT_REVIEWER_TYPE || 'claude';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Run ONE agent: (re)launch it fresh into `project`, inject `instruction`, then
// wait for it to write/modify files and settle. The handoff between agents is
// file-based (each reads what the last wrote under `project`).
async function runAgentStage({ name, type, project, instruction, waitMs }) {
  spawnSync('mkdir', ['-p', project]);
  spawnSync('tmux', ['kill-session', '-t', name], { encoding: 'utf8' }); // ensure --fresh
  const up = spawnSync(
    AGENTCHAT_BIN,
    ['up-v1', name, type, '--project', project, '--project-mode', 'symlink', '--fresh'],
    { encoding: 'utf8', cwd: AGENTCHAT_REPO, timeout: 60000 },
  );
  if (up.status !== 0) {
    throw new Error(
      `agentchat up-v1 ${name}/${type} failed (${up.status}): ${(up.stderr || up.stdout || '').slice(0, 300)}`,
    );
  }
  await sleep(ORCH_BOOT_MS);
  const before = listFilesUnder(project);
  const flat = instruction.replace(/\s+/g, ' ').trim();
  spawnSync('tmux', ['send-keys', '-t', name, '-l', flat], { encoding: 'utf8' });
  spawnSync('tmux', ['send-keys', '-t', name, 'Enter'], { encoding: 'utf8' });
  const deadline = Date.now() + (waitMs || ORCH_WAIT_MS);
  while (Date.now() < deadline) {
    await sleep(ORCH_POLL_MS);
    const after = listFilesUnder(project);
    const changed = Object.keys(after).filter((p) => before[p] === undefined || after[p] > before[p]);
    if (changed.length > 0) {
      const snap = { ...after };
      await sleep(ORCH_POLL_MS);
      const again = listFilesUnder(project);
      if (changed.every((p) => again[p] === snap[p])) return again;
    }
  }
  return listFilesUnder(project);
}

async function dispatchOrchestrateTeam(task) {
  const tmuxOk = spawnSync('which', ['tmux'], { encoding: 'utf8' }).status === 0;
  const agentChatOk = existsSync(AGENTCHAT_BIN);
  const coderOk = haveCliAgent(CODER_TYPE);
  const reviewerOk = haveCliAgent(REVIEWER_TYPE);
  if (!tmuxOk || !agentChatOk || !coderOk) {
    const r = await dispatchLlm(task);
    return { files: r.files, notes: `[agent-chat:team->fell-back-to-llm] ${r.notes}` };
  }

  const buildRoot = path.join(
    process.env.AGENTCHAT_ORCH_SCRATCH || '/tmp/openfab-agentchat-builds',
    `team-${Date.now()}`,
  );
  const td = task.target_dir;
  const checks = (task.acceptance || []).join('  AND  ');

  // 1) CODER implements.
  await runAgentStage({
    name: CODER_AGENT, type: CODER_TYPE, project: buildRoot,
    instruction:
      `You are the CODER on a two-person team. Build this and write ALL files under the absolute directory ${buildRoot}/ ` +
      `using absolute paths (e.g. ${buildRoot}/${td}/<file>) — nowhere else. The request: ${task.intent}. ` +
      (checks ? `It MUST satisfy these shell checks — cd ${buildRoot} and run them to confirm: ${checks}. ` : '') +
      `Write the files now, run the checks, then reply DONE.`,
  });

  // 2) REVIEWER critiques -> REVIEW.md (genuine second agent, ideally a different model).
  let reviewed = false;
  if (reviewerOk) {
    await runAgentStage({
      name: REVIEWER_AGENT, type: REVIEWER_TYPE, project: buildRoot,
      instruction:
        `You are the REVIEWER on a two-person team. The coder wrote software under ${buildRoot}/. ` +
        `Requirement: ${task.intent}. ` + (checks ? `It must satisfy: ${checks}. ` : '') +
        `Read every file under ${buildRoot}/, then write a concise critique to ${buildRoot}/REVIEW.md — a numbered list of concrete bugs, ` +
        `missing edge cases, and required fixes. Do NOT edit the code yourself. Then reply DONE.`,
    });
    reviewed = existsSync(path.join(buildRoot, 'REVIEW.md'));
  }

  // 3) CODER revises per the review.
  if (reviewed) {
    await runAgentStage({
      name: CODER_AGENT, type: CODER_TYPE, project: buildRoot,
      instruction:
        `You are the CODER. A reviewer left ${buildRoot}/REVIEW.md. Read it and the code under ${buildRoot}/, apply EVERY fix it lists, ` +
        (checks ? `then cd ${buildRoot} and re-run the checks to confirm they pass: ${checks}. ` : '') +
        `Edit the files in place under ${buildRoot}/, then reply DONE.`,
    });
  }

  // 4) Harvest the product (code under target_dir/) + REVIEW.md as the visible
  //    collaboration artifact, dropping any stray scratch files.
  const final = listFilesUnder(buildRoot);
  const files = {};
  for (const abs of Object.keys(final)) {
    const rel = path.relative(buildRoot, abs);
    if (!rel || rel.includes('..')) continue;
    if (!(rel.startsWith(`${td}/`) || rel === 'REVIEW.md')) continue;
    files[rel] = readFileSync(abs, 'utf8');
  }
  const fixed = validateFiles(files, task.target_dir);
  return {
    files: fixed,
    notes:
      `[agent-chat:team] ${CODER_TYPE} coder` +
      (reviewed ? ` + ${REVIEWER_TYPE} reviewer (critique→revise)` : ' (no reviewer)') +
      ` produced ${Object.keys(fixed).length} file(s)`,
  };
}

// ----- HTTP surface (the native dispatch endpoint) -----------------------
function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

const server = http.createServer(async (req, res) => {
  const send = (code, obj) => {
    const payload = JSON.stringify(obj);
    res.writeHead(code, {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(payload),
    });
    res.end(payload);
  };

  if (req.method === 'GET' && req.url === '/health') {
    return send(200, {
      ok: true,
      base: 'agent-chat',
      mode: MODE,
      llm: { endpoint: LLM_ENDPOINT, model: LLM_MODEL },
    });
  }
  if (req.method !== 'POST') {
    return send(405, { error: 'method not allowed; POST the dispatch JSON' });
  }

  let task;
  try {
    const raw = await readBody(req);
    task = JSON.parse(raw || '{}');
  } catch (e) {
    return send(400, { error: `invalid JSON body: ${e.message}` });
  }
  if (!task.intent || !task.target_dir) {
    return send(400, {
      error: 'dispatch body requires {intent, target_dir[, language, acceptance]}',
    });
  }
  if (!Array.isArray(task.acceptance)) task.acceptance = [];

  try {
    const result =
      MODE === 'team-native'
        ? await dispatchTeamNative(task)
        : MODE === 'team'
          ? await dispatchOrchestrateTeam(task)
          : MODE === 'orchestrate'
            ? await dispatchOrchestrate(task)
            : await dispatchLlm(task);
    // Final contract guard (defense in depth; R14).
    if (!result.files || Object.keys(result.files).length === 0) {
      return send(502, {
        error: 'agent-chat produced an empty manifest (rejected, not a pass)',
      });
    }
    return send(200, { files: result.files, notes: result.notes || '' });
  } catch (e) {
    // Surface the real cause to OpenFab's provenance instead of swallowing (R5).
    return send(502, {
      error: `agent-chat native dispatch failed: ${e.message}`,
    });
  }
});

server.listen(PORT, HOST, () => {
  process.stdout.write(
    `[openfab-agentchat] native base listening http://${HOST}:${PORT}  mode=${MODE}  model=${LLM_MODEL}\n` +
      `[openfab-agentchat] export OPENFAB_AGENTCHAT_URL=http://${HOST}:${PORT}/dispatch\n`,
  );
});
