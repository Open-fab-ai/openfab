"use strict";
// OpenFab web UI — talks to the JSON API in src/server.rs (same `ops` layer as the CLI).

const $ = (s) => document.querySelector(s);
const el = (t, c, h) => { const e = document.createElement(t); if (c) e.className = c; if (h != null) e.innerHTML = h; return e; };
const api = async (m, url, body) => {
  const r = await fetch(url, { method: m, headers: { "Content-Type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `${r.status}`);
  return j;
};
function toast(msg, err) { const t = el("div", "toast" + (err ? " err" : ""), msg); document.body.appendChild(t); setTimeout(() => t.remove(), 4200); }

let STATE = { runId: null, poll: null, lastSeq: 0, status: null, artifacts: null, verify: null, draft: null };

// ---------- init ----------
async function init() {
  $("#run").onclick = startRun;
  $("#addmaint").onclick = addMaintainer;
  $("#refine").onclick = refine;
  $("#reprobtn").onclick = reproduce;
  $("#runapp").onclick = runApp;
  $("#tryrun").onclick = () => tryRun();
  $("#trycmd").addEventListener("keydown", (e) => { if (e.key === "Enter") tryRun(); });
  $("#gate").onchange = updateGateHint; updateGateHint();
  $("#reqchanges").onclick = () => { $("#fbnote").scrollIntoView({ block: "center" }); $("#fbnote").focus(); };
  $("#rejectbtn").onclick = rejectRun;
  document.querySelectorAll(".step").forEach((s) => (s.onclick = () => showPhase(s.dataset.step)));
  document.querySelectorAll(".tab").forEach((t) => (t.onclick = () => selectTab(t.dataset.tab)));

  await Promise.all([loadBases(), loadForges(), loadMaintainers(), loadReputation()]);
  await ensureDefaultMaintainers();
}

async function loadBases() {
  const bases = await api("GET", "/api/bases");
  const sel = $("#base"); sel.innerHTML = "";
  bases.forEach((b) => { const o = el("option"); o.value = b.id; o.textContent = b.display; o._b = b; sel.appendChild(o); });
  sel.onchange = () => updateBaseBadge(bases);
  updateBaseBadge(bases);
}
function updateBaseBadge(bases) {
  const b = bases.find((x) => x.id === $("#base").value);
  $("#basebadge").innerHTML = b ? `<span class="badge ${b.runtime}">${b.runtime}</span>` : "";
  $("#basehint").textContent = b ? b.note : "";
}
async function loadForges() {
  const forges = await api("GET", "/api/forges");
  const sel = $("#forge"); sel.innerHTML = "";
  forges.forEach((f) => { const o = el("option"); o.value = f.id; o.textContent = f.display; o._f = f; sel.appendChild(o); });
  sel.onchange = () => updateForgeBadge(forges);
  updateForgeBadge(forges);
}
function updateForgeBadge(forges) {
  const f = forges.find((x) => x.id === $("#forge").value);
  $("#forgebadge").innerHTML = f ? `<span class="badge ${f.live ? "live" : "local"}">${f.live ? "live" : "local instance"}</span>` : "";
  $("#forgehint").textContent = f ? f.note : "";
}

// ---------- maintainers ----------
async function loadMaintainers() {
  const ms = await api("GET", "/api/maintainers");
  const box = $("#maintainers"); box.innerHTML = "";
  if (!ms.length) box.appendChild(el("span", "muted", "none yet — add at least 2 for the gate"));
  ms.forEach((m) => box.appendChild(el("span", "mtag", `${m.name} <span class="did">${shortDid(m.did)}</span>`)));
  return ms;
}
async function ensureDefaultMaintainers() {
  const ms = await api("GET", "/api/maintainers");
  // "me" = the solo developer's own identity; alice/bob = team reviewers.
  for (const name of ["me", "alice", "bob"]) if (!ms.find((m) => m.name === name)) await api("POST", "/api/maintainers", { name });
  await loadMaintainers();
}

const GATE_HINTS = {
  solo: "You build it, you ship it — one approval (yours), recorded & signed. Realistic for a developer on their own laptop.",
  team: "Two distinct maintainers must approve before it merges into the shared trusted repo.",
  crowd: "Contributions from people/agents you don't trust still flow in — but nothing merges without 2 maintainer approvals. The gate IS the trust.",
  none: "No human gate — provenance + machine acceptance still recorded. Fastest; least assurance.",
};
function updateGateHint() { $("#gatehint").textContent = GATE_HINTS[$("#gate").value] || ""; }
const approvalsNeeded = (mode) => ({ solo: 1, team: 2, crowd: 2, none: 0 }[mode] ?? 2);

async function addMaintainer() {
  const name = $("#newmaint").value.trim(); if (!name) return;
  await api("POST", "/api/maintainers", { name }); $("#newmaint").value = "";
  await loadMaintainers(); toast(`maintainer '${name}' registered`);
}


// ---------- run ----------
// The UI sends only the intent; the server has the LLM author the spec, then builds.
async function startRun() {
  const intent = $("#intent").value.trim();
  if (intent.length < 4) return toast("describe what you want to build first", true);
  resetFlow();
  $("#run").disabled = true; $("#run").innerHTML = '<span class="spin"></span> the LLM is authoring the spec & building…';
  try {
    const { run_id } = await api("POST", "/api/run", { intent, base: $("#base").value, forge: $("#forge").value, gate: $("#gate").value });
    STATE.runId = run_id; STATE.lastSeq = 0;
    setStatus("queued");
    startPolling();
  } catch (e) { toast(e.message, true); resetRunBtn(); }
}
function resetRunBtn() { $("#run").disabled = false; $("#run").innerHTML = "⚙ Fabricate trusted software"; }
function resetFlow() {
  $("#timeline").innerHTML = ""; $("#approvecard").classList.add("hidden"); $("#productcard").classList.add("hidden");
  $("#phasedetail").classList.add("hidden"); $("#phasedetail").innerHTML = "";
  $("#appframe").innerHTML = ""; $("#runappmsg").innerHTML = "";
  document.querySelectorAll(".step").forEach((s) => s.classList.remove("done", "active"));
  STATE.artifacts = null; STATE.verify = null;
}

function startPolling() {
  clearInterval(STATE.poll);
  STATE.poll = setInterval(tick, 650);
  tick();
}
async function tick() {
  if (!STATE.runId) return;
  try {
    const evs = await api("GET", `/api/runs/${STATE.runId}/events?since=${STATE.lastSeq}`);
    evs.forEach(addEvent);
    if (evs.length) STATE.lastSeq = evs[evs.length - 1].seq;
    const run = await api("GET", `/api/runs/${STATE.runId}`);
    setStatus(run.status || "running");
    if (["blocked", "accepted", "merged", "failed"].includes(run.status)) {
      clearInterval(STATE.poll); STATE.poll = null; resetRunBtn();
      onRunDone(run);
    }
  } catch (e) { /* transient while files are written */ }
}

function addEvent(ev) {
  const empty = $("#timeline .empty"); if (empty) empty.remove();
  const sub = ev.icon.trim().length > 2 || ev.icon.startsWith(" ");
  const item = el("div", "tl-item");
  item.append(el("div", "tl-ico", ev.icon), (() => {
    const b = el("div", "tl-body");
    b.append(el("div", "tl-msg" + (sub ? " sub" : ""), escapeHtml(ev.msg)));
    b.append(el("div", "tl-ts", ev.ts));
    return b;
  })());
  $("#timeline").appendChild(item);
  $("#timeline").scrollTop = $("#timeline").scrollHeight;
  advanceStepper(ev.msg);
}
function advanceStepper(msg) {
  const map = [["spec", /compiled|intent received/i], ["generate", /base '|implemented/i], ["verify", /acceptance/i], ["sign", /attestation|signed/i], ["gate", /trust gate|sign-off/i]];
  const steps = [...document.querySelectorAll(".step")];
  map.forEach(([k, re]) => { if (re.test(msg)) { const s = steps.find((x) => x.dataset.step === k); if (s) { steps.forEach((x) => x.classList.remove("active")); s.classList.add("active"); markPriorDone(steps, s); } } });
}
function markPriorDone(steps, cur) { let hit = false; steps.forEach((s) => { if (s === cur) hit = true; else if (!hit) s.classList.add("done"); }); }

// Click a workflow step → inspect exactly what that phase produced.
async function showPhase(step) {
  const pd = $("#phasedetail"); pd.classList.remove("hidden");
  if (!STATE.artifacts) { pd.innerHTML = `<div class="ph-h">${step}</div><div class="muted">Run a fabrication first — then each step reveals exactly what it produced.</div>`; return; }
  const a = STATE.artifacts, p = a.attestation.statement.predicate;
  let h = "";
  if (step === "spec") {
    h = `<div class="ph-h">📋 Spec — the contract compiled from your natural language</div>
      <div class="muted">Your intent becomes a versioned, machine-checkable spec. This exact spec was dispatched to the base and is committed with the run.</div>
      <pre class="code">${escapeHtml(JSON.stringify(a.spec, null, 2))}</pre>`;
  } else if (step === "generate") {
    h = `<div class="ph-h">🤖 Generate — what the agent authored</div>
      <div class="kv"><div class="k">base · model</div><div class="v">${p.agent.base} · ${p.agent.model}</div>
      <div class="k">runtime</div><div class="v">${a.run.base_runtime}</div>
      <div class="k">prompt sha256</div><div class="v">${p.prompt_sha256}</div></div>` +
      a.files.map((f) => `<div class="file-h">${f.path} · sha256 ${f.sha256.slice(0,16)}… · author <span class="tag-${f.author}">${f.author}</span></div>`).join("") +
      `<div class="muted">Full source is in the Software tab; run it in “Try the software”.</div>`;
  } else if (step === "verify") {
    h = `<div class="ph-h">🧪 Verify — the acceptance contract, executed in the sandbox</div>
      <div class="muted">“Acceptance” = the machine-checkable definition of done. Each id (a1, a2, …) is one criterion: a shell command that must exit 0. They are re-run on every reproduce.</div>
      <table class="rep"><tr><th>id</th><th>check (must exit 0)</th><th>result</th></tr>` +
      (a.run.acceptance || []).map((o) => `<tr><td class="mono">${o.id}</td><td class="mono">${escapeHtml(o.check)}</td><td>${o.passed ? "✅ pass" : "❌ fail (" + o.exit_code + ")"}</td></tr>`).join("") + `</table>`;
  } else if (step === "sign") {
    h = `<div class="ph-h">🔏 Sign — cryptographic provenance (in-toto/SLSA)</div>
      <div class="kv"><div class="k">payload sha256</div><div class="v">${a.attestation.payload_sha256}</div></div>
      <table class="rep"><tr><th>role</th><th>signer (did:key)</th><th>algo</th></tr>` +
      (a.attestation.signatures || []).map((s) => `<tr><td>${s.role}</td><td class="mono">${shortDid(s.keyid)}</td><td>${s.algo}</td></tr>`).join("") + `</table>`;
  } else if (step === "gate") {
    if (!STATE.verify) STATE.verify = await api("GET", `/api/runs/${STATE.runId}/verify`);
    const v = STATE.verify;
    h = `<div class="ph-h">🛡️ Gate — the trust decision (blocks merge until satisfied)</div>
      <div class="muted">conformant: <b>${v.conformant ? "yes" : "no"}</b> · accepted: <b>${v.accepted ? "yes" : "no"}</b> · merged: <b>${a.run.merged ? "yes" : "no"}</b></div>
      <div class="checklist">` + v.checks.map((c) => `<div class="${c.passed ? "ok" : "no"}">${c.id} — ${escapeHtml(c.detail)}</div>`).join("") + `</div>`;
  }
  pd.innerHTML = h;
}

function setStatus(st) {
  STATE.status = st;
  const p = $("#statuspill"); p.className = "pill " + st; p.textContent = st;
  if (st === "merged" || st === "accepted") document.querySelectorAll(".step").forEach((s) => s.classList.add("done"));
}

async function onRunDone(run) {
  if (run.status === "failed") { toast("run failed — see the timeline", true); return; }
  await loadArtifacts();          // load first so approval can show the approval count
  await showApproval(run);
  await loadReputation();
}

// ---------- approval ----------
async function showApproval(run) {
  const card = $("#approvecard"); card.classList.remove("hidden");
  const mode = run.gate_mode || "team";
  const needed = approvalsNeeded(mode);
  const signoffs = (STATE.artifacts && STATE.artifacts.attestation.statement.predicate.signoffs) || [];
  const have = signoffs.length;
  const signedNames = new Set(signoffs.map((s) => s.name));

  // Plain-English framing — what you're approving and why.
  $("#approveintro").innerHTML = mode === "solo"
    ? "You built it and tried it above. <b>Approving releases it</b> — the change merges into your <code>main</code> branch with its signed provenance and audit trail. Approve only if the software does what you asked; otherwise <b>Request changes</b> or <b>Reject</b>. (You're confirming it meets your intent — the cryptographic checks are automatic.)"
    : mode === "crowd"
    ? "This change may come from a contributor or agent you don't fully trust. <b>Two distinct maintainers</b> must confirm it does what was asked before it <b>merges into the trusted repo</b>. You're approving the result, not the cryptography (that's automatic)."
    : mode === "none"
    ? "No human gate is required by policy — the product is accepted on machine checks + provenance alone."
    : "<b>Two distinct maintainers</b> review and approve that the software meets intent before it <b>merges into main</b>. You're approving the result, not the cryptography (that's automatic).";

  // Status line.
  if (run.merged) $("#nofm").innerHTML = `✅ <b style="color:var(--ok)">Approved & released</b> — merged.`;
  else if (run.accepted) $("#nofm").innerHTML = `✅ <b style="color:var(--ok)">Approved.</b>`;
  else if (run.status === "rejected") $("#nofm").innerHTML = `⃠ <b style="color:var(--bad)">Rejected.</b> Refine it below, or start a new build.`;
  else if (needed === 0) $("#nofm").innerHTML = `Machine checks ✓ — no human approval required by policy.`;
  else $("#nofm").innerHTML = `Machine checks ✓ passed. <b>${have} of ${needed}</b> human approval${needed > 1 ? "s" : ""} — ${needed - have} more needed. <span class="muted">(${mode} policy)</span>`;

  // Approve buttons (solo = one "you" button; team/crowd = per-maintainer).
  const box = $("#signbtns"); box.innerHTML = "";
  const actionable = !run.accepted && !run.merged && run.status !== "rejected";
  if (actionable && needed > 0) {
    if (mode === "solo") {
      const b = el("button", "btn ok sm", "✓ Approve &amp; release");
      b.onclick = () => signoff("me", b); box.appendChild(b);
    } else {
      const ms = await api("GET", "/api/maintainers");
      ms.filter((m) => m.name !== "me").forEach((m) => {
        const done = signedNames.has(m.name);
        const b = el("button", "btn ok sm", done ? `✓ ${m.name} approved` : `✍ approve as ${m.name}`);
        if (done) b.disabled = true; else b.onclick = () => signoff(m.name, b);
        box.appendChild(b);
      });
    }
  }
  $("#rejectrow").style.display = actionable ? "flex" : "none";

  if (!STATE.verify) STATE.verify = await api("GET", `/api/runs/${STATE.runId}/verify`);
  renderGateChecks(STATE.verify);
}
function renderGateChecks(v) {
  const box = $("#gatechecks"); box.innerHTML = "";
  (v.checks || []).forEach((c) => box.appendChild(el("div", c.passed ? "ok" : "no", `${c.id} — ${escapeHtml(c.detail)}`)));
}
async function rejectRun() {
  try {
    const r = await api("POST", `/api/runs/${STATE.runId}/reject`);
    setStatus(r.status); STATE.verify = null; await showApproval(r);
    toast("Rejected. Tweak it via Refine, or edit the intent at the top + Fabricate for a different app.");
  } catch (e) { toast(e.message, true); }
}
async function signoff(name, btn) {
  btn.disabled = true; btn.innerHTML = '<span class="spin"></span>';
  try {
    const out = await api("POST", `/api/runs/${STATE.runId}/signoff`, { as: name });
    toast(out.merged ? "approved → released (merged) ✅" : out.accepted ? "approved ✅" : `${name} approved — more needed`);
    const run = await api("GET", `/api/runs/${STATE.runId}`);
    STATE.verify = null;
    setStatus(run.status); await loadArtifacts(); await showApproval(run); await loadReputation();
  } catch (e) { toast(e.message, true); btn.disabled = false; btn.textContent = `✍ approve as ${name}`; }
}
async function refine() {
  const note = $("#fbnote").value.trim(); if (!note) return toast("describe the change you want", true);
  const priorRun = STATE.runId;
  resetFlow();
  try {
    const { run_id } = await api("POST", `/api/runs/${priorRun}/feedback`, { note, base: $("#base").value });
    STATE.runId = run_id; STATE.lastSeq = 0; $("#fbnote").value = "";
    toast("refining → the LLM re-authors the spec & rebuilds (v→v+1)"); setStatus("queued"); startPolling();
  } catch (e) { toast(e.message, true); }
}

// ---------- artifacts ----------
async function loadArtifacts() {
  STATE.artifacts = await api("GET", `/api/runs/${STATE.runId}/artifacts`);
  $("#productcard").classList.remove("hidden");
  buildTryPresets();
  $("#tryout").style.display = "none";
  selectTab("code");
}

// Build clickable "try it" commands from the run's acceptance checks (always runnable),
// plus a couple of app-specific niceties for the temperature converter.
function buildTryPresets() {
  const a = STATE.artifacts; const box = $("#trypresets"); box.innerHTML = "";
  const cmds = [];
  const convert = (a.files.find((f) => f.path.endsWith("convert.py")) || {}).path;
  if (convert) { ["100 c2f", "32 f2c", "0 c2k", "--selftest"].forEach((x) => cmds.push(`python3 ${convert} ${x}`)); }
  (a.run.acceptance || []).forEach((o) => { if (!cmds.includes(o.check)) cmds.push(o.check); });
  cmds.slice(0, 8).forEach((c) => { const chip = el("span", "chip preset", escapeHtml(c)); chip.onclick = () => { $("#trycmd").value = c; tryRun(c); }; box.appendChild(chip); });
  $("#trycmd").value = cmds[0] || "";
}

// "Run the app": web app → open in browser; CLI → run a representative command.
async function runApp() {
  const btn = $("#runapp"), msg = $("#runappmsg");
  btn.disabled = true; btn.innerHTML = '<span class="spin"></span> starting…'; msg.innerHTML = "";
  try {
    const r = await api("POST", `/api/runs/${STATE.runId}/launch`);
    if (r.kind === "web") {
      // Embed it live (no popup blocker), plus a link to open full-screen + a stop control.
      msg.innerHTML = `🌐 current version running below at <a href="${r.url}" target="_blank" rel="noopener">${r.url}</a> · <a href="#" id="openapp">open full tab</a> · <a href="#" id="stopapp">stop</a><br><span class="muted">(any earlier tab you opened is now stopped — use this one)</span>`;
      // cache-bust so the iframe never shows a previously-cached version
      $("#appframe").innerHTML = `<iframe src="${r.url}?t=${Date.now()}" style="width:100%;height:460px;border:1px solid var(--line);border-radius:10px;background:#fff"></iframe>`;
      $("#appframe").scrollIntoView({ block: "nearest" });
      $("#openapp").onclick = (e) => { e.preventDefault(); window.open(r.url, "_blank"); };
      $("#stopapp").onclick = async (e) => { e.preventDefault(); await api("POST", `/api/runs/${STATE.runId}/stop`); $("#appframe").innerHTML = ""; msg.innerHTML = "stopped."; };
    } else if (r.kind === "web-failed") {
      msg.innerHTML = `⚠ ${escapeHtml(r.error)}. Try “run a custom command” below, or refine: “serve on the PORT env var”.`;
    } else {
      // CLI — run the first representative command and show output.
      msg.textContent = "This is a CLI (no web server) — running it in the sandbox:";
      const cmd = (STATE.artifacts.run.acceptance || [])[0]?.check || $("#trycmd").value;
      if (cmd) await tryRun(cmd); else msg.textContent = "Use “run a custom command” below.";
    }
  } catch (e) { msg.textContent = "error: " + e.message; }
  finally { btn.disabled = false; btn.innerHTML = "▶ Run the app"; }
}

async function tryRun(cmd) {
  cmd = (cmd || $("#trycmd").value || "").trim(); if (!cmd) return toast("type a command to run", true);
  const out = $("#tryout"); out.style.display = "block"; out.textContent = "running in sandbox…";
  try {
    const r = await api("POST", `/api/runs/${STATE.runId}/exec`, { cmd });
    const parts = [`$ ${r.cmd}`];
    if (r.stdout) parts.push(r.stdout.replace(/\n$/, ""));
    if (r.stderr) parts.push("[stderr] " + r.stderr.trim());
    parts.push(`— exit ${r.exit_code} ${r.exit_code === 0 ? "✓" : "✗"}`);
    out.textContent = parts.join("\n");
  } catch (e) { out.textContent = "error: " + e.message; }
}
function selectTab(name) {
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.tab === name));
  const a = STATE.artifacts; if (!a) return;
  const body = $("#tabbody"); body.innerHTML = "";
  if (name === "code") {
    if (!a.files.length) { body.appendChild(el("div", "empty", "no files")); return; }
    a.files.forEach((f) => {
      body.appendChild(el("div", "file-h", `${f.path}  ·  sha256 ${f.sha256.slice(0, 16)}…  ·  author: <span class="tag-${f.author}">${f.author}</span>`));
      body.appendChild(el("pre", "code", escapeHtml(f.contents)));
    });
  } else if (name === "prov") {
    body.appendChild(renderProvenance(a.attestation));
  } else if (name === "audit") {
    body.appendChild(el("div", "empty", "loading audit trail…"));
    loadAudit(body);
  } else if (name === "sbom") {
    body.appendChild(el("pre", "code", escapeHtml(JSON.stringify(a.sbom, null, 2))));
  } else if (name === "log") {
    body.appendChild(el("pre", "code", escapeHtml(a.timeline)));
  }
}

async function loadAudit(body) {
  let d;
  try { d = await api("GET", `/api/runs/${STATE.runId}/audit`); }
  catch (e) { body.innerHTML = `<div class="empty">audit unavailable: ${e.message}</div>`; return; }
  body.innerHTML = "";
  body.appendChild(el("div", "audit-note", "🛡 " + escapeHtml(d.compliance_note)));

  body.appendChild(el("div", "panel-h", `Git commit graph — ${d.forge_kind} · branch ${escapeHtml(d.branch)} · ${d.merged ? "merged ✓" : "open (gated)"}`));
  body.appendChild(el("pre", "code", escapeHtml(d.graph_ascii || "(no graph)")));

  body.appendChild(el("div", "panel-h", "Every action is a signed commit with provenance trailers:"));
  d.commits.forEach((c) => {
    const box = el("div", "commit");
    box.appendChild(el("div", "c-sub", escapeHtml(c.subject)));
    box.appendChild(el("div", "c-meta", `${c.short} · ${escapeHtml(c.author)} · ${c.date}` + (c.refs ? ` · <span class="c-refs">${escapeHtml(c.refs)}</span>` : "")));
    if (c.trailers.length) {
      const tr = el("div", "trailers");
      c.trailers.forEach(([k, v]) => tr.appendChild(el("span", "trailer", `<b>${escapeHtml(k)}</b> ${escapeHtml(v.length > 46 ? v.slice(0, 30) + "…" + v.slice(-8) : v)}`)));
      box.appendChild(tr);
    }
    body.appendChild(box);
  });

  body.appendChild(el("div", "panel-h", "Verify it independently — third-party tools, no OpenFab required:"));
  const tp = el("div", "tp");
  d.third_party.forEach((t) => {
    const row = el("div", "row");
    row.appendChild(el("div", "tool", escapeHtml(t.tool)));
    row.appendChild(el("div", "why", escapeHtml(t.purpose)));
    row.appendChild(el("pre", null, escapeHtml(t.cmd)));
    tp.appendChild(row);
  });
  body.appendChild(tp);
}
function renderProvenance(att) {
  const wrap = el("div");
  const p = (att.statement && att.statement.predicate) || {};
  const kv = el("div", "kv");
  const add = (k, v) => { kv.appendChild(el("div", "k", k)); kv.appendChild(el("div", "v", v)); };
  add("predicateType", att.statement?.predicateType || "");
  add("agent DID", (p.agent?.did) || "");
  add("base · model", `${p.agent?.base || ""} · ${p.agent?.model || ""}`);
  add("prompt sha256", (p.prompt_sha256 || "").slice(0, 32) + "…");
  add("acceptance", p.acceptance_passed ? "✅ passed" : "❌ failed");
  add("payload sha256", (att.payload_sha256 || "").slice(0, 32) + "…");
  const sigs = (att.signatures || []).map((s) => `${s.role}:${shortDid(s.keyid)}`).join("  ·  ");
  add("signatures", sigs);
  const auth = (p.generated || []).map((g) => `${g.path} [${g.lines}] <span class="tag-${g.author}">${g.author}</span>`).join("<br>");
  add("attribution", auth);
  const so = (p.signoffs || []).map((s) => `${s.name} (${shortDid(s.did)})`).join("  ·  ") || "—";
  add("human sign-offs", so);
  wrap.appendChild(kv);
  const det = el("details"); det.appendChild(el("summary", "muted", "raw attestation JSON"));
  det.appendChild(el("pre", "code", escapeHtml(JSON.stringify(att, null, 2)))); wrap.appendChild(det);
  return wrap;
}

// ---------- reproduce ----------
async function reproduce() {
  const btn = $("#reprobtn"); btn.disabled = true; btn.innerHTML = '<span class="spin"></span> reproducing…';
  try {
    const r = await api("POST", `/api/runs/${STATE.runId}/reproduce`);
    const row = $("#reprorow"); row.innerHTML = "";
    row.appendChild(el("div", "big " + (r.reproducible ? "ok" : "no"), r.reproducible ? "REPRODUCIBLE ✓" : "NOT REPRODUCIBLE ✗"));
    const crit = el("div", "crit");
    crit.innerHTML =
      `signature <b class="${r.signature_valid ? "ok" : "no"}">${r.signature_valid ? "valid" : "INVALID"}</b> · ` +
      `source <b class="${r.source_identical ? "ok" : "no"}">${r.source_identical ? "bit-identical" : "DIFFERS"}</b> (${r.files_checked} files) · ` +
      `acceptance <b class="${r.all_acceptance_passed ? "ok" : "no"}">${r.all_acceptance_passed ? "all pass" : "FAILED"}</b>`;
    row.appendChild(crit);
    const tbl = el("table", "rep");
    tbl.innerHTML = "<tr><th>check</th><th>result</th><th>exit</th></tr>" +
      r.checks.map((c) => `<tr><td class="mono">${escapeHtml(c.check)}</td><td>${c.passed ? "✅ pass" : "❌ fail"}</td><td>${c.exit_code}</td></tr>`).join("");
    row.appendChild(tbl);
    toast(r.reproducible ? "independently reproduced ✓" : "reproduction mismatch", !r.reproducible);
  } catch (e) { toast(e.message, true); }
  finally { btn.disabled = false; btn.innerHTML = "⟳ Reproduce & verify (sovereign proof)"; }
}

// ---------- reputation ----------
async function loadReputation() {
  try {
    const r = await api("GET", "/api/reputation");
    const box = $("#reputation");
    if (!r.agents.length) { box.innerHTML = '<div class="empty">Run a fabrication to populate.</div>'; return; }
    const t = el("table", "rep");
    t.innerHTML = "<tr><th>identity</th><th>authored</th><th>accepted</th><th>signoffs</th></tr>" +
      r.agents.map((a) => `<tr><td class="mono">${shortDid(a.did)}</td><td>${a.authored}</td><td>${a.accepted}</td><td>${a.signoffs_given}</td></tr>`).join("");
    box.innerHTML = ""; box.appendChild(t);
  } catch (e) {}
}

// ---------- util ----------
function shortDid(d) { return d && d.length > 22 ? d.slice(0, 14) + "…" + d.slice(-4) : (d || ""); }
function escapeHtml(s) { return (s || "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); }

init();
