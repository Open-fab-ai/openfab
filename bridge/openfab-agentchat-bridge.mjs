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
  const tree = (body.existing_tree || []).map((p) => `  ${p}`).join('\n');
  let modeBlock;
  if (body.mode === 'workspace') {
    modeBlock = [
      `MODE: SHARED WORKSPACE. The repository is on THIS machine at:`,
      `  ${body.repo_path}`,
      `cd into it. READ whatever files you need for full context (imports, callers, types).`,
      `EDIT the allowed file(s) IN PLACE there. Keep changes minimal; do NOT replace`,
      `Cargo.toml/package.json wholesale or drop dependencies. Run the bound tests in that repo`,
      `(e.g. cargo test <filter>) until they pass.`,
      `When done, reply task_result with status:"completed", model, and`,
      `changed_paths: ["<relpath you edited>", ...]  (NO files map needed — your edits are on disk).`,
    ].join('\n');
  } else if (body.mode === 'refactor') {
    modeBlock = [
      `MODE: REFACTOR an EXISTING repo. The current source is in schema.payload.existing_files`,
      `(a map of relpath → full content)${body.existing_truncated ? ' — TRUNCATED; the full file tree is below' : ''}.`,
      `Modify those files in place. Return ONLY the files you actually changed or added, with`,
      `their FULL new content. Do NOT regenerate the project, replace Cargo.toml/package.json`,
      `wholesale, or drop existing dependencies/modules — that breaks the build.`,
      tree ? `REPO FILE TREE:\n${tree}` : '',
    ].filter(Boolean).join('\n');
  } else {
    modeBlock = `MODE: GREENFIELD — emit a complete, buildable project at the target dir.`;
  }
  const allow = (body.allow || []).map((p) => `  - ${p}`).join('\n');
  const requirements = (body.requirements || '').trim();
  return [
    `═══ OpenFab task briefing ═══  (spec: ${body.spec_ref})`,
    `You are the implementer. Everything you need is below — you should NOT have to reverse-`,
    `engineer the task from the codebase. Read this briefing first, then do exactly this.`,
    ``,
    `WHAT TO BUILD (intent):\n${body.intent}`,
    ``,
    requirements ? `WHY / FULL REQUIREMENTS (the agreed brief):\n${requirements}\n` : '',
    allow ? `FILES YOU MAY CHANGE (and ONLY these):\n${allow}\n` : '',
    `HOW TO WORK:`,
    modeBlock,
    ``,
    `LANGUAGE: ${body.language || 'any'}  TARGET DIR: ${body.target_dir || 'app'}/`,
    assumptions ? `CONSTRAINTS / DECISIONS:\n${assumptions}` : '',
    checks ? `DONE WHEN these bound tests pass (write them with these EXACT names):\n${checks}` : '',
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
      schema: { kind: 'task_request', version: 1, payload: {
        task_id: acTaskId, room: body.room,
        mode: body.mode || 'greenfield',
        repo_path: body.repo_path || null,        // shared-workspace mode: edit in place here
        existing_files: body.existing_files || {}, // mount mode: code shipped over the bridge
        existing_tree: body.existing_tree || [],
      } },
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
    if (m?.schema?.kind === 'task_result' && p?.task_id === acTaskId && (p.files || p.attachments || p.changed_paths)) {
      const files = p.files || {};
      const file_hashes = {};
      for (const [fp, content] of Object.entries(files)) file_hashes[fp] = sha256(content);
      // workspace mode: the agent edited in place and reports which paths it changed
      return { files, file_hashes, model: p.model || '', prompt: p.prompt || '', changed_paths: p.changed_paths || [] };
    }
  }
  return null;
}

async function getTask(id) {
  const acTaskId = id.startsWith('of-') ? id.slice(3) : id;
  // The implementer reports via a `task_result` message, not by transitioning the task —
  // so harvest the message FIRST, regardless of the agent-chat task status.
  const harvested = harvestResult(acTaskId);
  if (harvested && (Object.keys(harvested.files).length || (harvested.changed_paths || []).length)) {
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

// Resolve the agent-chat GROUP that maps to an OpenFab post target so the message reaches the
// Matrix room (Robrix). `target` may be a room id (`!room:server`), a project name, or a group.
// Chain: project → bound room (via /api/rooms) → group (from the message store's sourceRoom).
async function resolveGroup(target) {
  let room = target && target.startsWith('!') ? target : null;
  if (!room && target) {
    try {
      const list = await (await fetch(`${OPENFAB_URL}/api/rooms`)).json();
      const b = (list || []).find((x) => x.project === target) || (list || [])[0];
      if (b) room = b.room;
    } catch { /* fall through */ }
  }
  if (room && MESSAGES_FILE) {
    try {
      let msgs = JSON.parse(fs.readFileSync(MESSAGES_FILE, 'utf8'));
      if (!Array.isArray(msgs)) msgs = msgs?.messages || [];
      for (let i = msgs.length - 1; i >= 0; i--) {
        if ((msgs[i].sourceRoom || msgs[i].source_room) === room && msgs[i].group) {
          return msgs[i].group;
        }
      }
    } catch { /* fall through */ }
  }
  return process.env.BRIDGE_POST_GROUP || null;
}

// Post OpenFab notifications straight into the Matrix room via the bot (which is a room member),
// so they reach Robrix reliably — the agent-chat group/puppet path can't relay `openfab-bridge`
// (it has no Matrix puppet in the room). Resolve the room from the project binding.
const MX_HS = process.env.MATRIX_HOMESERVER || '';
const MX_USER = process.env.MATRIX_BOT_USERNAME || '';
const MX_PASS = process.env.MATRIX_BOT_PASSWORD || '';
let _mxToken = null;
let _mxTxn = 0;
async function matrixToken() {
  if (_mxToken) return _mxToken;
  if (!MX_HS || !MX_USER || !MX_PASS) return null;
  try {
    const r = await fetch(`${MX_HS}/_matrix/client/v3/login`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ type: 'm.login.password', identifier: { type: 'm.id.user', user: MX_USER }, password: MX_PASS }),
    });
    _mxToken = (await r.json()).access_token || null;
  } catch { _mxToken = null; }
  return _mxToken;
}
async function resolveRoomId(target) {
  if (target && target.startsWith('!')) return target;
  try {
    const list = await (await fetch(`${OPENFAB_URL}/api/rooms`)).json();
    const b = (list || []).find((x) => x.project === target) || (list || [])[0];
    return b ? b.room : null;
  } catch { return null; }
}
async function postMessage(target, msg) {
  const token = await matrixToken();
  const room = await resolveRoomId(target);
  if (token && room) {
    const txn = `of-${Date.now()}-${_mxTxn++}`;
    const res = await fetch(
      `${MX_HS}/_matrix/client/v3/rooms/${encodeURIComponent(room)}/send/m.room.message/${txn}`,
      { method: 'PUT', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ msgtype: 'm.text', body: msg }) },
    );
    if (res.ok) return;
    if (res.status === 401) _mxToken = null; // token expired — drop so next call re-logs in
    log(`matrix post to ${room} failed: ${res.status}`);
  }
  // Fallback: drop it into the agent-chat store (visible in the monitor, not the Matrix room).
  await acFetch('/api/messages', {
    method: 'POST',
    body: JSON.stringify({ from: SELF, to: ASSIGNEE, type: 'inform', summary: 'OpenFab', full: msg,
      schema: { kind: 'note', version: 1, payload: { room: target } } }),
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

// --- Phase 3: dispatch an agent-spec AI review to the reviewer agent + harvest decisions ---
const BRIDGE_REVIEWER = process.env.BRIDGE_REVIEWER || 'wf_reviewer';
const reviews = new Map(); // review_id → { acTaskId }
async function createReview(body) {
  const created = await acFetch('/api/tasks', {
    method: 'POST',
    body: JSON.stringify({
      title: `OpenFab review: ${body.spec_ref}`,
      description: 'Review AI-pending agent-spec scenarios',
      priority: 'p1', granularity: 'task', assignee: BRIDGE_REVIEWER,
      created_by: SELF, labels: ['openfab', 'review'],
    }),
  });
  const acTaskId = created?.task?.id;
  if (!acTaskId) throw new Error('agent-chat did not return a task id');
  await acFetch('/api/messages', {
    method: 'POST',
    body: JSON.stringify({
      from: SELF, to: BRIDGE_REVIEWER, type: 'request',
      summary: `Review ${body.spec_ref}`,
      full: 'OpenFab review request — decide each AI-pending scenario by reading the code.',
      schema: { kind: 'review_request', version: 1, payload: {
        review_id: acTaskId, spec_ref: body.spec_ref,
        requests: body.requests || [], files: body.files || {} } },
    }),
  });
  reviews.set(`rv-${acTaskId}`, { acTaskId });
  return `rv-${acTaskId}`;
}
function harvestReview(acTaskId) {
  if (!MESSAGES_FILE) return null;
  let msgs;
  try { msgs = JSON.parse(fs.readFileSync(MESSAGES_FILE, 'utf8')); } catch { return null; }
  if (!Array.isArray(msgs)) msgs = msgs?.messages || [];
  for (let i = msgs.length - 1; i >= 0; i--) {
    const p = msgs[i]?.schema?.payload;
    if (msgs[i]?.schema?.kind === 'review_result' && p?.review_id === acTaskId && p.decisions) {
      return { decisions: p.decisions };
    }
  }
  return null;
}
async function getReview(id) {
  const acTaskId = id.startsWith('rv-') ? id.slice(3) : id;
  const h = harvestReview(acTaskId);
  if (h && Array.isArray(h.decisions)) return { status: 'done', decisions: h.decisions };
  return { status: 'running' };
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

// Submit a build the agent-chat team produced in-room → OpenFab imports it and runs it through
// the gate (verify → sign → conformance → N-of-M sign-off). The single convergence point: any
// build path (dashboard or room) ends at OpenFab's gate. Pre-ingest the spec via submitDoc.
async function submitBuild({ room, id, files, model, builder, gate, project }) {
  const res = await fetch(`${OPENFAB_URL}/api/import-build`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ room, id, files, model, builder, gate, project }),
  });
  const text = await res.text();
  let run_id;
  try { run_id = JSON.parse(text).run_id; } catch { /* surface raw below */ }
  return { ok: res.ok, status: res.status, run_id, body: text.slice(0, 300) };
}

// --- B2: relay a Robrix approval to OpenFab's sign-off / reject API ---
// Resolve which OpenFab project a room is bound to (so a run's sign-off scopes to the right
// workspace). Cached briefly; falls back to the default project when unbound.
let _roomMapAt = 0, _roomMap = {};
async function roomProject(room) {
  if (!room) return null;
  if (Date.now() - _roomMapAt > 3000) {
    try {
      const r = await fetch(`${OPENFAB_URL}/api/rooms`);
      const list = await r.json();
      _roomMap = Object.fromEntries((list || []).map((b) => [b.room, b.project]));
      _roomMapAt = Date.now();
    } catch { /* keep stale map */ }
  }
  return _roomMap[room] || null;
}

async function relayApproval(run, mxid, action, project) {
  const q = project && project !== 'default' ? `?project=${encodeURIComponent(project)}` : '';
  if (action === 'reject') {
    const res = await fetch(`${OPENFAB_URL}/api/runs/${encodeURIComponent(run)}/reject${q}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ by: mxid }),
    });
    return { ok: res.ok, status: res.status };
  }
  // approve / sign → N-of-M sign-off as the mapped maintainer (OpenFab rejects unmapped mxids)
  const res = await fetch(`${OPENFAB_URL}/api/runs/${encodeURIComponent(run)}/signoff${q}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ mxid }),
  });
  const text = await res.text();
  return { ok: res.ok, status: res.status, body: text.slice(0, 300) };
}

// B2 poller: scan the message store for Matrix-origin approval commands and relay them.
// Bind the message's own Matrix room to an OpenFab project (Phase 2.1 #3, from-the-room UX):
// a user types `/bind <project>` (or `bind <project>`) in the room; the room id comes from
// the server-attested `source_room`, so no curl / no knowing the room id is needed.
async function relayBind(room, project) {
  const res = await fetch(`${OPENFAB_URL}/api/rooms`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ room, project }),
  });
  return { ok: res.ok, status: res.status };
}

const seenCmds = new Set();
async function pollApprovals() {
  if (!MESSAGES_FILE) return;
  let msgs;
  try {
    msgs = JSON.parse(fs.readFileSync(MESSAGES_FILE, 'utf8'));
  } catch {
    return;
  }
  if (!Array.isArray(msgs)) msgs = msgs?.messages || [];
  const approveRe = /^\s*(approve|sign|reject)\s+(\S+)/i;
  const bindRe = /^\s*\/?bind\s+(\S+)/i;
  for (const m of msgs) {
    // Only honor commands that genuinely came through Matrix (a server-attested sender) —
    // do NOT trust a self-declared sender_mxid on a non-matrix message (privilege escalation).
    const mxid = m.sender_mxid || m.senderMxid;
    if (!mxid || m.source !== 'matrix') continue;
    // Strip leading Matrix mention markup (`[@coordinator](https://matrix.to/#/@…)`) so a
    // natural "@coordinator approve <run>" is caught by the IDENTITY-CHECKED relay here, rather
    // than falling through to an agent that would forge the sign-off via the CLI.
    const body = (m.full || m.summary || '').replace(/\[@[^\]]*\]\([^)]*\)/g, '').trim();
    const room = m.source_room || m.sourceRoom;
    const key = m.id || `${mxid}:${body}`;
    if (seenCmds.has(key)) continue;

    const bindM = bindRe.exec(body);
    if (bindM && room) {
      seenCmds.add(key);
      try {
        const r = await relayBind(room, bindM[1]);
        log(`bound room ${room} → project ${bindM[1]} (by ${mxid}) → ${r.status}`);
      } catch (e) {
        log(`room bind failed for ${room}: ${e.message}`);
      }
      continue;
    }

    const match = approveRe.exec(body);
    if (!match) continue;
    seenCmds.add(key);
    const action = match[1].toLowerCase();
    const run = match[2];
    try {
      // Scope the sign-off to the room's bound project (so a dashboard-built run in the
      // `openfab` project is signed there, not in `default`).
      const project = await roomProject(room);
      const r = await relayApproval(run, mxid, action, project);
      log(`relayed ${action} ${run} by ${mxid} (project ${project || 'default'}) → ${r.status}`);
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
    // Room-built code → OpenFab gate — POST /submit-build {room, id, files, model, gate} → {run_id}
    if (req.method === 'POST' && url.pathname === '/submit-build') {
      const body = await readBody(req);
      const r = await submitBuild(body);
      return send(res, r.ok ? 200 : 502, r);
    }
    // Phase 3: agent-spec AI review — POST /review {spec_ref, requests, files, room} → {review_id}
    if (req.method === 'POST' && url.pathname === '/review') {
      const body = await readBody(req);
      const review_id = await createReview(body);
      return send(res, 200, { review_id });
    }
    const reviewMatch = url.pathname.match(/^\/review\/(.+)$/);
    if (req.method === 'GET' && reviewMatch) {
      return send(res, 200, await getReview(decodeURIComponent(reviewMatch[1])));
    }

    send(res, 404, { error: 'not found' });
  } catch (e) {
    log('error', e);
    send(res, 500, { error: String(e.message || e) });
  }
});

// Make the room↔project binding "live": actively harvest specs the coordinator produces in
// its workspace (native issue-workflow writes `specs/*.spec.md` + `issues/*.md` there but never
// calls the API) and ingest them into the bound OpenFab project, so issues built in Robrix show
// up on the dashboard without anyone pushing. Set BRIDGE_COORDINATOR_WS (comma-separated paths).
const COORD_WS = (process.env.BRIDGE_COORDINATOR_WS || '')
  .split(',').map((s) => s.trim()).filter(Boolean);
const seenSpecs = new Set();
async function harvestProject() {
  // explicit override, else the single bound project (unambiguous in the common case).
  if (process.env.BRIDGE_HARVEST_PROJECT) return process.env.BRIDGE_HARVEST_PROJECT;
  try {
    const r = await fetch(`${OPENFAB_URL}/api/rooms`);
    const list = await r.json();
    if (Array.isArray(list) && list.length === 1) return list[0].project;
  } catch { /* fall through */ }
  return null;
}
async function harvestCoordinatorSpecs() {
  if (!COORD_WS.length) return;
  const project = await harvestProject();
  if (!project) return; // ambiguous (0 or >1 bindings) → set BRIDGE_HARVEST_PROJECT
  for (const ws of COORD_WS) {
    const specDir = path.join(ws, 'specs');
    let entries;
    try { entries = fs.readdirSync(specDir); } catch { continue; }
    for (const f of entries) {
      if (!f.endsWith('.spec.md')) continue;
      const full = path.join(specDir, f);
      let mtime;
      try { mtime = fs.statSync(full).mtimeMs; } catch { continue; }
      const key = `${full}:${mtime}`;
      if (seenSpecs.has(key)) continue;
      seenSpecs.add(key);
      // id = filename without `task-` prefix and `.spec.md` suffix.
      const id = f.replace(/^task-/, '').replace(/\.spec\.md$/, '');
      const num = (id.match(/^(\d+)/) || [])[1];
      let spec_md, requirements_md = '';
      try { spec_md = fs.readFileSync(full, 'utf8'); } catch { continue; }
      if (num) {
        for (const sub of ['issues', 'docs/designs', 'docs/plans']) {
          try {
            const hit = fs.readdirSync(path.join(ws, sub)).find((x) => x.startsWith(`${num}-`));
            if (hit) { requirements_md = fs.readFileSync(path.join(ws, sub, hit), 'utf8'); break; }
          } catch { /* keep looking */ }
        }
      }
      try {
        const res = await fetch(`${OPENFAB_URL}/api/ingest`, {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ id, project, spec_md, requirements_md }),
        });
        log(`harvested spec '${id}' → project ${project} → ${res.status}`);
      } catch (e) {
        log(`harvest ingest failed for ${id}: ${e.message}`);
      }
    }
  }
}

server.listen(PORT, '127.0.0.1', () => {
  log(`listening on http://127.0.0.1:${PORT}  → agent-chat ${AC}  assignee=${ASSIGNEE}`);
  log(`approval relay → OpenFab ${OPENFAB_URL} (polling every ${APPROVAL_POLL_MS}ms)`);
  if (!MESSAGES_FILE) log('WARNING: set AGENTCHAT_DIR (the agent-chat repo path) so the bridge can harvest implementer results');
  if (COORD_WS.length) log(`spec harvest watching: ${COORD_WS.join(', ')}`);
  registerSelf();
  setInterval(() => {
    pollApprovals().catch((e) => log(`approval poll error: ${e.message}`));
    harvestCoordinatorSpecs().catch((e) => log(`harvest error: ${e.message}`));
  }, APPROVAL_POLL_MS);
});
