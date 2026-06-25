#!/usr/bin/env node
// OpenFab ↔ agent-chat Bridge (Phase 1) — zero-dependency Node ESM.
//
// Absorbs the async↔blocking impedance between OpenFab (blocking HTTP, single binary, no
// tokio) and the agent-chat backend + Matrix (async). OpenFab drives; the agent-chat
// implementer agent (in a Matrix room) only does the "implement" segment.
//
//   OpenFab  ──blocking HTTP──▶  THIS BRIDGE  ──HTTP──▶  agent-chat backend (:8090) ──▶ Matrix room
//                              (this file)                  /api/tasks, /api/messages, /api/dm/...
//
// OpenFab-facing API (consumed by src/adapters/bridge_client.rs):
//   POST /tasks      {spec_ref,intent,target_dir,language,acceptance,assumptions,context,room}
//                    → {task_id}
//   GET  /tasks/:id  → {status:"running|done|failed", files:{path:content},
//                       file_hashes:{path:sha256}, model, prompt, error?}
//   POST /post       {room,msg} → {ok:true}
//   GET  /healthz    → {ok:true}
//
// Agent-side RESULT CONTRACT (the implementer agent must follow this so OpenFab can sign
// bit-identical bytes — see bridge/README.md and the issue-workflow skill):
//   When done, the implementer posts a message with
//     schema = { kind:"task_result", version:1, payload:{
//        task_id, status:"completed", model, prompt, files:{ "<relpath>":"<full content>" } } }
//   (Alternatively, files may be delivered as attachments; payload.files takes precedence.)
//
// Config (env):
//   BRIDGE_PORT            (default 8077)        — OpenFab-facing port
//   AGENTCHAT_URL          (default http://127.0.0.1:8090)
//   AGENTCHAT_API_TOKEN    operator Bearer token for the agent-chat backend
//   BRIDGE_ASSIGNEE        (default wf_implementer) — agent that implements
//   BRIDGE_OPERATOR        (default operator)    — DM history owner to harvest results from
//   BRIDGE_ATTACH_DIR      (default <AGENTCHAT>/data/message-attachments)
//
// Run:  node bridge/openfab-agentchat-bridge.mjs

import http from 'node:http';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileP = promisify(execFile);
// OpenFab serve URL — the Bridge relays Robrix approvals to its sign-off API (Phase 2 B2).
const OPENFAB_URL = (process.env.OPENFAB_URL || 'http://127.0.0.1:8787').replace(/\/$/, '');
const APPROVAL_POLL_MS = Number(process.env.BRIDGE_APPROVAL_POLL_MS || 5000);

const PORT = Number(process.env.BRIDGE_PORT || 8077);
const AC = (process.env.AGENTCHAT_URL || 'http://127.0.0.1:8090').replace(/\/$/, '');
const TOKEN = process.env.AGENTCHAT_API_TOKEN || '';
const ASSIGNEE = process.env.BRIDGE_ASSIGNEE || 'wf_implementer';
const OPERATOR = process.env.BRIDGE_OPERATOR || 'operator';
// The implementer replies with a `task_result` message addressed to SELF. The HTTP
// `/api/dm/:name/history` endpoint doesn't surface messages to a service agent, so we read
// the message store on disk (same host) and match by task_id — robust and restart-safe.
// Set AGENTCHAT_DIR (the agent-chat repo) or AGENTCHAT_MESSAGES_FILE on each machine; there
// is no portable default. When unset, result harvesting is disabled (warned at startup).
const MESSAGES_FILE =
  process.env.AGENTCHAT_MESSAGES_FILE ||
  (process.env.AGENTCHAT_DIR
    ? path.join(process.env.AGENTCHAT_DIR, 'data', 'messages.json')
    : null);

const SELF = 'openfab-bridge'; // the sender identity; must be a registered agent-chat agent
const sha256 = (s) => crypto.createHash('sha256').update(s, 'utf8').digest('hex');
const log = (...a) => console.error('[bridge]', ...a);

// Register our sender identity so agent-chat accepts our messages (idempotent).
async function registerSelf() {
  try {
    await acFetch('/api/agents', {
      method: 'POST',
      body: JSON.stringify({ name: SELF, role: 'bridge', type: 'service' }),
    });
    log(`registered sender agent "${SELF}"`);
  } catch (e) {
    log(`registerSelf warning: ${e.message}`);
  }
}

// task_id → { acRoomTaskId, room, prompt } (in-memory; restart loses in-flight tasks)
const tasks = new Map();

function authHeaders() {
  const h = { 'Content-Type': 'application/json' };
  if (TOKEN) h.Authorization = `Bearer ${TOKEN}`;
  return h;
}

async function acFetch(path, opts = {}) {
  const res = await fetch(`${AC}${path}`, { headers: authHeaders(), ...opts });
  const text = await res.text();
  let json;
  try { json = text ? JSON.parse(text) : {}; } catch { json = { raw: text }; }
  if (!res.ok) throw new Error(`agent-chat ${path} → ${res.status}: ${text.slice(0, 300)}`);
  return json;
}

// Build the natural-language instruction the implementer agent reads from the room.
function instruction(body) {
  const checks = (body.acceptance || []).map((c, i) => `  ${i + 1}. ${c}`).join('\n');
  const assumptions = (body.assumptions || []).map((a) => `  - ${a}`).join('\n');
  return [
    `OpenFab task — implement the spec below. Spec: ${body.spec_ref}`,
    ``,
    `INTENT:\n${body.intent}`,
    ``,
    `LANGUAGE: ${body.language || 'any'}  TARGET DIR: ${body.target_dir || 'app'}/`,
    assumptions ? `CONSTRAINTS / DECISIONS:\n${assumptions}` : '',
    checks ? `BOUND TEST SCENARIOS (your code + tests must make these pass):\n${checks}` : '',
    ``,
    `When done, reply with a message whose schema.payload =`,
    `{ kind:"task_result", task_id:"<id>", status:"completed", model:"<model>",`,
    `  prompt:"<the prompt you worked from>", files:{ "<relpath>":"<full file content>" } }`,
  ].filter(Boolean).join('\n');
}

async function createTask(body) {
  // 1. create the agent-chat task
  const created = await acFetch('/api/tasks', {
    method: 'POST',
    body: JSON.stringify({
      title: `OpenFab: ${body.spec_ref}`,
      description: body.intent,
      priority: 'p1',
      granularity: 'task',
      assignee: ASSIGNEE,
      created_by: SELF,
      labels: ['openfab'],
    }),
  });
  const acTaskId = created?.task?.id;
  if (!acTaskId) throw new Error('agent-chat did not return a task id');

  // 2. post the instruction into the room (the implementer reads it from inbox)
  const prompt = instruction(body);
  await acFetch('/api/messages', {
    method: 'POST',
    body: JSON.stringify({
      from: SELF,
      to: ASSIGNEE,
      type: 'request',
      summary: `Implement ${body.spec_ref}`,
      full: prompt,
      schema: { kind: 'task_request', version: 1, payload: { task_id: acTaskId, room: body.room } },
    }),
  });

  const id = `of-${acTaskId}`;
  tasks.set(id, { acTaskId, room: body.room, prompt });
  return id;
}

// Find the implementer's `task_result` for an agent-chat task id by reading the message
// store on disk and matching `schema.payload.task_id`. Robust to message-routing quirks
// and to bridge restarts (no in-memory state needed).
function harvestResult(acTaskId) {
  if (!MESSAGES_FILE) { log('harvest: AGENTCHAT_DIR/AGENTCHAT_MESSAGES_FILE not set — cannot harvest results'); return null; }
  let msgs;
  try {
    msgs = JSON.parse(fs.readFileSync(MESSAGES_FILE, 'utf8'));
  } catch (e) {
    log(`harvest: cannot read ${MESSAGES_FILE}: ${e.message}`);
    return null;
  }
  if (!Array.isArray(msgs)) msgs = msgs?.messages || [];
  // newest last; scan from the end for the latest matching result
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    const p = m?.schema?.payload;
    if (m?.schema?.kind === 'task_result' && p?.task_id === acTaskId && (p.files || p.attachments)) {
      const files = p.files || {};
      const file_hashes = {};
      for (const [fp, content] of Object.entries(files)) file_hashes[fp] = sha256(content);
      return { files, file_hashes, model: p.model || '', prompt: p.prompt || '' };
    }
  }
  return null;
}

async function getTask(id) {
  const acTaskId = id.startsWith('of-') ? id.slice(3) : id;
  // The implementer reports via a `task_result` message, not by transitioning the task —
  // so harvest the message FIRST, regardless of the agent-chat task status.
  const harvested = harvestResult(acTaskId);
  if (harvested && Object.keys(harvested.files).length) {
    return { status: 'done', ...harvested };
  }
  // No result yet: surface failed only if the task itself failed; else keep running.
  try {
    const acTask = await acFetch(`/api/tasks/${encodeURIComponent(acTaskId)}`);
    const status = acTask?.status || acTask?.task?.status || 'running';
    if (status === 'failed') return { status: 'failed', error: 'agent-chat task failed' };
  } catch {
    /* transient: keep running */
  }
  return { status: 'running' };
}

async function postMessage(room, msg) {
  await acFetch('/api/messages', {
    method: 'POST',
    body: JSON.stringify({
      from: SELF,
      to: ASSIGNEE,
      type: 'inform',
      summary: 'OpenFab',
      full: msg,
      schema: { kind: 'note', version: 1, payload: { room } },
    }),
  });
}

// --- C2: agent status (proxy agent-chat) ---
async function listAgents() {
  return acFetch('/api/agents');
}

// --- C3: tmux monitor (capture a live agent session pane) ---
async function peekAgent(name, lines) {
  // Only allow well-formed agent names (no shell/tmux injection).
  if (!/^[A-Za-z0-9_.-]+$/.test(name)) throw new Error(`bad agent name: ${name}`);
  const n = Math.min(Math.max(parseInt(lines, 10) || 60, 1), 400);
  try {
    const { stdout } = await execFileP('tmux', ['capture-pane', '-t', name, '-p']);
    const all = stdout.split('\n');
    return { agent: name, lines: all.slice(Math.max(0, all.length - n)) };
  } catch (e) {
    return { agent: name, lines: [], error: `no live tmux session: ${e.message}` };
  }
}

// --- #3: submit a coordinator's finalized docs to OpenFab (scoped to the room's project) ---
async function submitDoc({ room, id, requirements_md, spec_md, project }) {
  const res = await fetch(`${OPENFAB_URL}/api/ingest`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ room, id, requirements_md, spec_md, project }),
  });
  const text = await res.text();
  return { ok: res.ok, status: res.status, body: text.slice(0, 300) };
}

// --- B2: relay a Robrix approval to OpenFab's sign-off / reject API ---
async function relayApproval(run, mxid, action) {
  if (action === 'reject') {
    const res = await fetch(`${OPENFAB_URL}/api/runs/${encodeURIComponent(run)}/reject`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ by: mxid }),
    });
    return { ok: res.ok, status: res.status };
  }
  // approve / sign → N-of-M sign-off as the mapped maintainer (OpenFab rejects unmapped mxids)
  const res = await fetch(`${OPENFAB_URL}/api/runs/${encodeURIComponent(run)}/signoff`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ mxid }),
  });
  const text = await res.text();
  return { ok: res.ok, status: res.status, body: text.slice(0, 300) };
}

// B2 poller: scan the message store for Matrix-origin approval commands and relay them.
const seenApprovals = new Set();
async function pollApprovals() {
  if (!MESSAGES_FILE) return;
  let msgs;
  try {
    msgs = JSON.parse(fs.readFileSync(MESSAGES_FILE, 'utf8'));
  } catch {
    return;
  }
  if (!Array.isArray(msgs)) msgs = msgs?.messages || [];
  const re = /^\s*(approve|sign|reject)\s+(\S+)/i;
  for (const m of msgs) {
    // Only honor approvals that genuinely came through Matrix (a server-attested sender) —
    // do NOT trust a self-declared sender_mxid on a non-matrix message (privilege escalation).
    const mxid = m.sender_mxid || m.senderMxid;
    if (!mxid || m.source !== 'matrix') continue;
    const body = m.full || m.summary || '';
    const match = re.exec(body);
    if (!match) continue;
    const key = m.id || `${mxid}:${body}`;
    if (seenApprovals.has(key)) continue;
    seenApprovals.add(key);
    const action = match[1].toLowerCase();
    const run = match[2];
    try {
      const r = await relayApproval(run, mxid, action);
      log(`relayed ${action} ${run} by ${mxid} → ${r.status}`);
    } catch (e) {
      log(`approval relay failed for ${run}: ${e.message}`);
    }
  }
}

function send(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { 'Content-Type': 'application/json' });
  res.end(body);
}

function readBody(req) {
  return new Promise((resolve) => {
    let data = '';
    req.on('data', (c) => (data += c));
    req.on('end', () => {
      try { resolve(data ? JSON.parse(data) : {}); } catch { resolve({}); }
    });
  });
}

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, `http://localhost:${PORT}`);
    if (req.method === 'GET' && url.pathname === '/healthz') return send(res, 200, { ok: true });

    if (req.method === 'POST' && url.pathname === '/tasks') {
      const body = await readBody(req);
      const id = await createTask(body);
      return send(res, 200, { task_id: id });
    }

    const taskMatch = url.pathname.match(/^\/tasks\/(.+)$/);
    if (req.method === 'GET' && taskMatch) {
      const r = await getTask(decodeURIComponent(taskMatch[1]));
      return send(res, 200, r);
    }

    if (req.method === 'POST' && url.pathname === '/post') {
      const body = await readBody(req);
      await postMessage(body.room, body.msg);
      return send(res, 200, { ok: true });
    }

    // C2: agent status
    if (req.method === 'GET' && url.pathname === '/agents') {
      return send(res, 200, await listAgents());
    }
    // C3: tmux monitor — GET /agents/:name/peek?lines=N
    const peekMatch = url.pathname.match(/^\/agents\/([^/]+)\/peek$/);
    if (req.method === 'GET' && peekMatch) {
      const r = await peekAgent(decodeURIComponent(peekMatch[1]), url.searchParams.get('lines'));
      return send(res, 200, r);
    }
    // B2: explicit approval relay — POST /approve {run, mxid, action}
    if (req.method === 'POST' && url.pathname === '/approve') {
      const body = await readBody(req);
      const r = await relayApproval(body.run, body.mxid, (body.action || 'approve').toLowerCase());
      return send(res, r.ok ? 200 : 502, r);
    }
    // #3: coordinator submits docs — POST /submit-doc {room, id, requirements_md, spec_md, project}
    if (req.method === 'POST' && url.pathname === '/submit-doc') {
      const body = await readBody(req);
      const r = await submitDoc(body);
      return send(res, r.ok ? 200 : 502, r);
    }

    send(res, 404, { error: 'not found' });
  } catch (e) {
    log('error', e);
    send(res, 500, { error: String(e.message || e) });
  }
});

server.listen(PORT, '127.0.0.1', () => {
  log(`listening on http://127.0.0.1:${PORT}  → agent-chat ${AC}  assignee=${ASSIGNEE}`);
  log(`approval relay → OpenFab ${OPENFAB_URL} (polling every ${APPROVAL_POLL_MS}ms)`);
  if (!MESSAGES_FILE) log('WARNING: set AGENTCHAT_DIR (the agent-chat repo path) so the bridge can harvest implementer results');
  registerSelf();
  setInterval(() => {
    pollApprovals().catch((e) => log(`approval poll error: ${e.message}`));
  }, APPROVAL_POLL_MS);
});
