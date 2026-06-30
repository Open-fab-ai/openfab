"use strict";
// OpenFab web UI — talks to the JSON API in src/server.rs (same `ops` layer as the CLI).

const $ = (s) => document.querySelector(s);
const el = (t, c, h) => { const e = document.createElement(t); if (c) e.className = c; if (h != null) e.innerHTML = h; return e; };
const api = async (m, url, body) => {
  const r = await fetch(url, { method: m, headers: { "Content-Type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) { const e = new Error(j.error || j.hint || `${r.status}`); e.status = r.status; e.body = j; throw e; }
  return j;
};
function toast(msg, err) { const t = el("div", "toast" + (err ? " err" : ""), msg); document.body.appendChild(t); setTimeout(() => t.remove(), 4200); }

let STATE = { runId: null, poll: null, lastSeq: 0, status: null, artifacts: null, verify: null, draft: null };

// ---------- init ----------
async function init() {
  $("#run").onclick = () => startRun(false);
  $("#addmaint").onclick = addMaintainer;
  $("#refine").onclick = refine;
  $("#reprobtn").onclick = reproduce;
  $("#openartifacts").onclick = openRunArtifacts;
  $("#dopenartifacts").onclick = openRunArtifacts;
  $("#runapp").onclick = runApp;
  $("#tryrun").onclick = () => tryRun();
  $("#trycmd").addEventListener("keydown", (e) => { if (e.key === "Enter") tryRun(); });
  $("#gate").onchange = updateGateHint; updateGateHint();
  $("#mode").onchange = updateModeHint; updateModeHint();
  $("#promote").onclick = promoteDraft;
  $("#draftrun").onclick = runDraftApp;
  $("#drefine").onclick = () => refineDraft(false);
  $("#reqchanges").onclick = () => { $("#fbnote").scrollIntoView({ block: "center" }); $("#fbnote").focus(); };
  $("#rejectbtn").onclick = rejectRun;
  document.querySelectorAll(".step").forEach((s) => (s.onclick = () => showPhase(s.dataset.step)));
  document.querySelectorAll(".tab").forEach((t) => (t.onclick = () => selectTab(t.dataset.tab)));
  document.querySelectorAll(".card.collapsible > h2").forEach((h) =>
    (h.onclick = () => h.parentElement.classList.toggle("collapsed")));
  $("#settingsbtn").onclick = () => toggleDrawer(true);
  $("#drawerclose").onclick = () => toggleDrawer(false);
  $("#drawerscrim").onclick = () => toggleDrawer(false);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") toggleDrawer(false); });

  await Promise.all([loadBases(), loadForges(), loadModels(), loadMaintainers(), loadReputation(), loadApps()]);
  await ensureDefaultMaintainers();
}

// Populate both model pickers from the configured Ollama endpoint (key stays server-side).
async function loadModels() {
  let models = [], err = null;
  try { const r = await api("GET", "/api/models"); models = r.models || []; err = r.error; }
  catch (e) { err = e.message; }
  for (const id of ["#authormodel", "#basemodel"]) {
    const sel = $(id); const keep = sel.value;
    sel.innerHTML = '<option value="">default</option>' + models.map((m) => `<option value="${m}">${m}</option>`).join("");
    if (keep) sel.value = keep;
  }
  $("#modelhint").textContent = models.length
    ? `${models.length} models available · empty = each side's configured default. Applies to LLM-driven generation (bridged, agentscope, agent-chat llm-mode); claude & agent-chat orchestrate/team run on their own CLI.`
    : (err ? `model list unavailable: ${err}` : "no models configured (set OPENFAB_OLLAMA_URL/KEY)");
}

async function loadBases() {
  const bases = await api("GET", "/api/bases");
  const sel = $("#base"); sel.innerHTML = "";
  bases.forEach((b) => { const o = el("option"); o.value = b.id; o.textContent = b.display; o._b = b; sel.appendChild(o); });
  // Default base: user preference (Settings) → codex → first available.
  const pref = localStorage.getItem("openfab_default_base");
  const def = (pref && bases.some((b) => b.id === pref)) ? pref
    : (bases.some((b) => b.id === "codex") ? "codex" : (bases[0] && bases[0].id));
  if (def) sel.value = def;
  sel.onchange = () => updateBaseBadge(bases);
  updateBaseBadge(bases);
  renderDefaultBaseSetting(bases, def);
}
// Settings → Default base: persist to localStorage and apply to the main selector.
function renderDefaultBaseSetting(bases, current) {
  const ds = $("#defaultbase"); if (!ds) return;
  ds.innerHTML = "";
  bases.forEach((b) => { const o = el("option"); o.value = b.id; o.textContent = b.display; ds.appendChild(o); });
  ds.value = current || "";
  ds.onchange = () => {
    localStorage.setItem("openfab_default_base", ds.value);
    const sel = $("#base"); sel.value = ds.value; updateBaseBadge(bases);
    toast(`default base set to ${ds.options[ds.selectedIndex].text}`);
  };
}
function updateBaseBadge(bases) {
  const b = bases.find((x) => x.id === $("#base").value);
  $("#basebadge").innerHTML = b ? `<span class="badge ${b.runtime}">${b.runtime === "native" ? "● live" : "○ " + b.runtime}</span>` : "";
  $("#basehint").textContent = b ? b.note : "";
  $("#baseprompt").classList.add("hidden");
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
async function startRun(allowBridged) {
  const intent = $("#intent").value.trim();
  if (intent.length < 4) return toast("describe what you want to build first", true);
  resetFlow();
  $("#baseprompt").classList.add("hidden");
  $("#run").disabled = true; $("#run").innerHTML = '<span class="spin"></span> the LLM is authoring the spec & building…';
  try {
    const { run_id } = await api("POST", "/api/run", { intent, base: $("#base").value, forge: $("#forge").value, gate: $("#gate").value, mode: $("#mode").value, allow_bridged: !!allowBridged, author_model: $("#authormodel").value, base_model: $("#basemodel").value });
    STATE.runId = run_id; STATE.lastSeq = 0;
    // Wizard: fold the build step to a summary so the live workflow is the only thing in focus.
    setCollapsed("#buildcard", `<b>${escapeHtml(intent.slice(0, 60))}</b> · ${escapeHtml($("#base").value)} · ${escapeHtml($("#mode").value)}`);
    setCollapsed("#flowcard", null);
    setStatus("queued");
    startPolling();
  } catch (e) {
    if (e.body && e.body.error_kind === "base_unavailable") { showBasePrompt(e.body, () => startRun(true)); resetRunBtn(); return; }
    toast(e.message, true); resetRunBtn();
  }
}

// The chosen base's native runtime isn't running. Offer to launch it (if OpenFab bundles a
// launcher) or to run with the bridged LLM stand-in — never substitute silently (R14).
// `onBridged` re-issues the original action with allow_bridged=true.
function showBasePrompt(info, onBridged) {
  const p = $("#baseprompt"); p.classList.remove("hidden");
  const launchBtn = info.launchable
    ? `<button class="btn small" id="bp-launch">▶ Launch ${info.display}</button>`
    : `<span class="muted">No bundled launcher — start its adapter, then retry.</span>`;
  p.innerHTML =
    `<div class="bp-title">⚠ ${info.display} is not running</div>` +
    `<div class="muted">${info.hint}</div>` +
    `<div class="bp-actions">${launchBtn}` +
    `<button class="btn small ghost" id="bp-bridge">Use bridged stand-in (OpenFab LLM, not the real base)</button></div>` +
    `<div class="bp-log" id="bp-log"></div>`;
  $("#bp-bridge").onclick = () => { p.classList.add("hidden"); onBridged(); };
  if (info.launchable) $("#bp-launch").onclick = async () => {
    const btn = $("#bp-launch"); btn.disabled = true; btn.innerHTML = '<span class="spin"></span> launching… (~30s)';
    try {
      const out = await api("POST", `/api/base/${info.base}/launch`);
      if (out.reachable) { $("#bp-log").textContent = "✅ " + out.detail; await loadBases(); p.classList.add("hidden"); startRun(false); }
      else { $("#bp-log").textContent = "⚠ " + out.detail; btn.disabled = false; btn.innerHTML = `▶ Launch ${info.display}`; }
    } catch (err) { $("#bp-log").textContent = "✖ " + err.message; btn.disabled = false; btn.innerHTML = `▶ Launch ${info.display}`; }
  };
}
function resetRunBtn() { $("#run").disabled = false; $("#run").innerHTML = "⚙ Fabricate trusted software"; }
function resetFlow() {
  $("#timeline").innerHTML = ""; $("#approvecard").classList.add("hidden"); $("#productcard").classList.add("hidden");
  $("#draftcard").classList.add("hidden"); $("#draftframe").innerHTML = "";
  $("#phasedetail").classList.add("hidden"); $("#phasedetail").innerHTML = "";
  $("#appframe").innerHTML = ""; $("#runappmsg").innerHTML = "";
  document.querySelectorAll(".step").forEach((s) => s.classList.remove("done", "active"));
  STATE.artifacts = null; STATE.verify = null;
  setCollapsed("#buildcard", null); setCollapsed("#flowcard", null); setCollapsed("#productcard", null);
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
    if (["blocked", "accepted", "merged", "failed", "draft"].includes(run.status)) {
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
  const a = STATE.artifacts;
  // A draft has no attestation yet (sign/gate only run on release). The predicate may be absent.
  const signed = a.attestation && a.attestation.statement;
  const p = signed ? a.attestation.statement.predicate : null;
  const draftNote = `<div class="muted">This is a <b>draft</b> — the trust ceremony (sign + gate) hasn't run yet. <b>Promote to a signed release</b> to produce in-toto/SLSA provenance.</div>`;
  let h = "";
  if (step === "spec") {
    h = `<div class="ph-h">📋 Spec — the contract compiled from your natural language</div>
      <div class="muted">Your intent becomes a versioned, machine-checkable spec. This exact spec was dispatched to the base and is committed with the run.</div>
      <pre class="code">${escapeHtml(JSON.stringify(a.spec, null, 2))}</pre>`;
  } else if (step === "generate") {
    h = `<div class="ph-h">🤖 Generate — what the agent authored</div>
      <div class="kv"><div class="k">base · model</div><div class="v">${p ? p.agent.base + " · " + p.agent.model : a.run.base_name}</div>
      <div class="k">runtime</div><div class="v">${a.run.base_runtime}</div>` +
      (p ? `<div class="k">prompt sha256</div><div class="v">${p.prompt_sha256}</div></div>` : `</div>`) +
      (a.prompt
        ? `<details style="margin-top:10px"><summary class="muted" style="cursor:pointer">▸ show the exact generation prompt</summary><pre class="code" style="margin-top:8px; white-space:pre-wrap; max-height:260px; overflow:auto">${escapeHtml(a.prompt)}</pre><div class="muted" style="margin-top:4px">Local run-state — the signed BOM stores only its sha256 (privacy/portability).</div></details>`
        : "") +
      (a.files.length
        ? a.files.map((f) => `<div class="file-h">${f.path} · sha256 ${f.sha256.slice(0,16)}… · author <span class="tag-${f.author}">${f.author}</span></div>`).join("")
        : `<div class="muted">Files are committed to the draft branch <span class="mono">${a.run.branch}</span>. The signed file manifest (sha256 + author per file) is produced on release.</div>`) +
      `<div class="muted">Run it in “Run the app”.</div>`;
  } else if (step === "verify") {
    h = `<div class="ph-h">🧪 Verify — the acceptance contract, executed in the sandbox</div>
      <div class="muted">“Acceptance” = the machine-checkable definition of done. Each id (a1, a2, …) is one criterion: a shell command that must exit 0. They are re-run on every reproduce.</div>
      <table class="rep"><tr><th>id</th><th>check (must exit 0)</th><th>result</th></tr>` +
      (a.run.acceptance || []).map((o) => `<tr><td class="mono">${o.id}</td><td class="mono">${escapeHtml(o.check)}</td><td>${o.passed ? "✅ pass" : "❌ fail (" + o.exit_code + ")"}</td></tr>`).join("") + `</table>`;
  } else if (step === "sign") {
    if (!signed) { h = `<div class="ph-h">🔏 Sign — cryptographic provenance (in-toto/SLSA)</div>${draftNote}`; }
    else h = `<div class="ph-h">🔏 Sign — cryptographic provenance (in-toto/SLSA)</div>
      <div class="kv"><div class="k">payload sha256</div><div class="v" title="${escapeHtml(JSON.stringify(a.attestation.statement, null, 2)).slice(0, 1400)}">${a.attestation.payload_sha256}</div></div>
      <table class="rep"><tr><th>role</th><th>signer (did:key)</th><th>algo</th></tr>` +
      (a.attestation.signatures || []).map((s) => `<tr><td>${s.role}</td><td class="mono">${shortDid(s.keyid)}</td><td>${s.algo}</td></tr>`).join("") + `</table>`;
  } else if (step === "gate") {
    if (!signed) { pd.innerHTML = `<div class="ph-h">🛡️ Gate — the trust decision</div>${draftNote}`; return; }
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
  // Keep the collapsed Live-workflow summary in sync after approval changes the status
  // (e.g. blocked → merged once the N-of-M gate is satisfied).
  const sum = $("#flowcard") && $("#flowcard").querySelector(".cardsummary");
  if (sum && $("#flowcard").classList.contains("collapsed"))
    sum.innerHTML = `✓ <b>${escapeHtml(st)}</b> · spec · generate · verify · sign · gate <span class="muted">(click to inspect)</span>`;
}

async function onRunDone(run) {
  loadApps();   // refresh the app list whenever a build finishes
  if (run.status === "failed") { toast("run failed — see the timeline", true); return; }
  // Wizard: fold the finished workflow to a summary line so the product/approve step is the focus.
  setCollapsed("#flowcard", `✓ <b>${escapeHtml(run.status)}</b> · spec · generate · verify · sign · gate <span class="muted">(click to inspect)</span>`);
  if (run.status === "draft") { await loadArtifacts(); showDraft(run); return; }   // un-attested fast loop — no provenance/gate, but spec/acceptance are inspectable
  await loadArtifacts();          // load first so approval can show the approval count
  await showApproval(run);
  await loadReputation();
}

// ---------- draft (fast, un-attested) ----------
const MODE_HINTS = {
  release: "Full trust ceremony: author spec → build → run acceptance → sign an in-toto/SLSA attestation → open a gated PR → block on N-of-M sign-off. The trusted checkpoint.",
  draft: "Fast inner loop: generate + run acceptance only. NO signature, gate, PR or provenance — iterate freely, nothing heavy fires. Promote to a signed release when ready.",
};
function updateModeHint() { $("#modehint").textContent = MODE_HINTS[$("#mode").value] || ""; }

function showDraft(run) {
  $("#productcard").classList.add("hidden");
  $("#approvecard").classList.add("hidden");
  const card = $("#draftcard"); card.classList.remove("hidden");
  const ok = run.acceptance_passed;
  $("#draftmsg").innerHTML =
    `⚡ <b>Draft built</b> on <code>${escapeHtml(run.branch)}</code> — acceptance ` +
    `<b class="${ok ? "ok" : "no"}">${ok ? "PASSED" : "FAILED"}</b>. ` +
    `<b>Un-attested</b>: no signature, no gate, no provenance yet.`;
  $("#promote").disabled = !ok;
  $("#promote").title = ok ? "" : "fix acceptance before promoting (no vacuous promotion)";
  card.scrollIntoView({ block: "nearest" });
}

async function runDraftApp() {
  const btn = $("#draftrun"), frame = $("#draftframe");
  btn.disabled = true; btn.innerHTML = '<span class="spin"></span> starting…';
  try {
    const r = await api("POST", `/api/runs/${STATE.runId}/launch`);
    if (r.kind === "web") {
      window.open(r.url + "?t=" + Date.now(), "_blank", "noopener");
      frame.innerHTML =
        `<div class="hint">🌐 running in a new tab → <a href="${r.url}" target="_blank" rel="noopener">${r.url}</a> · <a href="#" id="dreopen">re-open</a> · <a href="#" id="dstopapp">stop</a></div>`;
      $("#dreopen").onclick = (e) => { e.preventDefault(); window.open(r.url + "?t=" + Date.now(), "_blank", "noopener"); };
      $("#dstopapp").onclick = async (e) => { e.preventDefault(); await api("POST", `/api/runs/${STATE.runId}/stop`); frame.innerHTML = "stopped."; };
    } else if (r.kind === "web-failed") {
      frame.innerHTML = `<div class="hint">⚠ ${escapeHtml(r.error)}</div>`;
    } else {
      frame.innerHTML = `<div class="hint">This is a CLI (no web server) — it ran in the sandbox. Promote it for the full “Try the software” surface.</div>`;
    }
  } catch (e) { frame.innerHTML = `<div class="hint">error: ${escapeHtml(e.message)}</div>`; }
  finally { btn.disabled = false; btn.innerHTML = "▶ Run the app"; }
}

async function promoteDraft() {
  const draftId = STATE.runId;
  $("#promote").disabled = true; $("#promote").innerHTML = '<span class="spin"></span> promoting…';
  try {
    const { run_id } = await api("POST", `/api/runs/${draftId}/promote`);
    toast("promoting → full ceremony (sign + gate + provenance)…");
    resetFlow();
    STATE.runId = run_id; STATE.lastSeq = 0; setStatus("queued"); startPolling();
  } catch (e) {
    toast(e.message, true);
    $("#promote").disabled = false; $("#promote").innerHTML = "✅ Promote to signed release →";
  }
}

async function refineDraft(allowBridged) {
  const note = $("#dfbnote").value.trim(); if (!note) return toast("describe the change you want", true);
  const priorRun = STATE.runId;
  resetFlow();
  try {
    const { run_id } = await api("POST", `/api/runs/${priorRun}/feedback`, { note, base: $("#base").value, mode: "draft", allow_bridged: !!allowBridged, author_model: $("#authormodel").value, base_model: $("#basemodel").value });
    STATE.runId = run_id; STATE.lastSeq = 0; $("#dfbnote").value = "";
    toast("re-drafting → the LLM re-authors the spec & rebuilds (v→v+1)"); setStatus("queued"); startPolling();
  } catch (e) {
    if (e.body && e.body.error_kind === "base_unavailable") { STATE.runId = priorRun; showBasePrompt(e.body, () => refineDraft(true)); return; }
    toast(e.message, true);
  }
}

// ---------- approval ----------
async function showApproval(run) {
  const card = $("#approvecard"); card.classList.remove("hidden");
  const mode = run.gate_mode || "team";
  const needed = approvalsNeeded(mode);
  const att = STATE.artifacts && STATE.artifacts.attestation;
  const signoffs = (att && att.statement && att.statement.predicate.signoffs) || [];
  // Count DISTINCT signers (the gate counts distinct maintainer DIDs, not raw records) so
  // a double sign-off can't read as "2 of 1".
  const have = new Set(signoffs.map((s) => s.did)).size;
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
  else $("#nofm").innerHTML = `Machine checks ✓ passed. <b>${have} of ${needed}</b> human approval${needed > 1 ? "s" : ""} — ${Math.max(0, needed - have)} more needed. <span class="muted">(${mode} policy)</span>`;

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
    // Once the gate is satisfied, fold the product step to a summary for a cleaner finish.
    if (run.status === "merged" || run.status === "accepted") {
      const n = (STATE.artifacts && STATE.artifacts.files || []).length;
      setCollapsed("#productcard", `✓ <b>${escapeHtml(run.status)}</b> · ${n} file(s) · provenance signed <span class="muted">(click to inspect)</span>`);
    }
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
  renderExplorer();
  // Bring the just-revealed product step into view and focus its primary action so
  // the user doesn't have to scroll-hunt for it after generation completes.
  requestAnimationFrame(() => {
    $("#productcard").scrollIntoView({ behavior: "smooth", block: "start" });
    const r = $("#runapp"); if (r) r.focus({ preventScroll: true });
  });
}

function toggleDrawer(open) {
  $("#settingsdrawer").classList.toggle("hidden", !open);
  $("#drawerscrim").classList.toggle("hidden", !open);
}

// Wizard collapse: fold a finished step to a one-line summary; null summary expands it.
function setCollapsed(cardId, summaryHtml) {
  const c = $(cardId); if (!c) return;
  if (summaryHtml == null) { c.classList.remove("collapsed"); return; }
  let s = c.querySelector(".cardsummary");
  if (!s) { s = document.createElement("div"); s.className = "cardsummary"; c.querySelector("h2").after(s); }
  s.innerHTML = summaryHtml;
  c.classList.add("collapsed");
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
      // Open the running app in a SEPARATE browser tab (keeps the OpenFab tab clean for
      // the demo), with controls to re-open or stop it.
      window.open(r.url + "?t=" + Date.now(), "_blank", "noopener");
      $("#appframe").innerHTML = "";
      msg.innerHTML = `🌐 running in a new tab → <a href="${r.url}" target="_blank" rel="noopener">${r.url}</a> · <a href="#" id="openapp">re-open</a> · <a href="#" id="stopapp">stop</a>`;
      $("#openapp").onclick = (e) => { e.preventDefault(); window.open(r.url + "?t=" + Date.now(), "_blank", "noopener"); };
      $("#stopapp").onclick = async (e) => { e.preventDefault(); await api("POST", `/api/runs/${STATE.runId}/stop`); msg.innerHTML = "stopped."; };
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
// ---------- artifact bundle explorer (tree + detail) ----------
let ARTNODES = {};
function fileIcon(p) {
  const e = (p.split(".").pop() || "").toLowerCase();
  return e === "md" ? "📝" : e === "json" ? "🔧" : e === "html" ? "🌐"
    : e === "js" ? "📜" : e === "py" ? "🐍" : e === "css" ? "🎨" : "📄";
}
function artNode(tree, id, icon, label, tag, child, fn) {
  const n = el("div", "tnode" + (child ? " child" : ""));
  n.innerHTML = `<span class="ticon">${icon}</span><span class="tlabel">${escapeHtml(label)}</span>` + (tag ? `<span class="ttag">${tag}</span>` : "");
  n.onclick = () => selectArt(id);
  tree.appendChild(n); ARTNODES[id] = { el: n, fn };
}
function selectArt(id) {
  Object.values(ARTNODES).forEach((o) => o.el.classList.remove("sel"));
  const o = ARTNODES[id]; if (!o) return;
  o.el.classList.add("sel"); o.fn();
}
function renderExplorer() {
  const a = STATE.artifacts; if (!a) return;
  const tree = $("#arttree"); tree.innerHTML = ""; ARTNODES = {};
  const fl = el("div", "tfolder"); fl.textContent = `Software · ${a.files.length} file(s)`; tree.appendChild(fl);
  a.files.forEach((f, i) => artNode(tree, "file" + i, fileIcon(f.path), f.path.split("/").pop(),
    `<span class="tag-${f.author}">${f.author}</span>`, true, () => renderFile(f)));
  if (a.attestation) artNode(tree, "aibom", "📄", "AI-BOM", "", false, () => renderAiBom(a.attestation));
  if (a.sbom) artNode(tree, "sbom", "📄", "SBOM", "", false, () => renderSbom(a.sbom));
  artNode(tree, "audit", "📜", "Audit trail", "", false, () => { const d = $("#artdetail"); d.innerHTML = '<div class="empty">loading audit trail…</div>'; loadAudit(d); });
  artNode(tree, "log", "📋", "Decision log", "", false, () => renderLog(a.timeline));
  selectArt(a.files.length ? "file0" : "aibom");
}

// Pretty | Raw toggle. `rawText` is the exact stored bytes — the signature covers these.
function viewToggle(detail, prettyFn, rawText) {
  const tg = el("div", "viewtoggle");
  const bP = el("button", null, "Pretty"), bR = el("button", null, "Raw");
  const view = el("div");
  const show = (pretty) => {
    bP.classList.toggle("on", pretty); bR.classList.toggle("on", !pretty); view.innerHTML = "";
    view.appendChild(pretty ? prettyFn() : el("pre", "code", escapeHtml(rawText)));
  };
  bP.onclick = () => show(true); bR.onclick = () => show(false);
  tg.append(bP, bR); detail.appendChild(tg); detail.appendChild(view); show(true);
}

function renderFile(f) {
  const d = $("#artdetail"); d.innerHTML = "";
  d.appendChild(el("div", "file-h", `${f.path}  ·  sha256 ${f.sha256.slice(0, 16)}…  ·  author: <span class="tag-${f.author}">${f.author}</span>`));
  const ext = (f.path.split(".").pop() || "").toLowerCase();
  if (ext === "md") {
    viewToggle(d, () => { const m = el("div", "md"); m.innerHTML = mdToHtml(f.contents); return m; }, f.contents);
  } else if (ext === "json") {
    let pretty; try { pretty = JSON.stringify(JSON.parse(f.contents), null, 2); } catch { pretty = f.contents; }
    viewToggle(d, () => el("pre", "code", escapeHtml(pretty)), f.contents);
  } else {
    d.appendChild(el("pre", "code", escapeHtml(f.contents)));
  }
}
function renderAiBom(att) {
  const d = $("#artdetail"); d.innerHTML = "";
  const p = (att.statement && att.statement.predicate) || {};
  const gen = p.generated || []; const ai = gen.filter((g) => g.author === "ai").length;
  const pct = gen.length ? Math.round((ai / gen.length) * 100) : 0;
  d.appendChild(el("div", "artclaim",
    `<b>${gen.length} file(s)</b> · ${pct}% AI-authored by <b>${escapeHtml(p.agent?.model || "?")}</b> · ` +
    `acceptance ${p.acceptance_passed ? "✅ passed" : "❌ failed"} · ${(p.signoffs || []).length} human sign-off(s)`));
  d.appendChild(renderProvenance(att));   // KV summary + raw attestation JSON (in <details>)
}
function renderSbom(sbom) {
  const d = $("#artdetail"); d.innerHTML = "";
  // Count components across formats: SPDX (files/packages) or CycloneDX (components).
  const comps = sbom.files || sbom.components || sbom.packages || sbom.artifacts || [];
  const cnt = Array.isArray(comps) ? comps.length : 0;
  const fmt = sbom.spdxVersion || (sbom.bomFormat ? `${sbom.bomFormat} ${sbom.specVersion || ""}`.trim() : "");
  d.appendChild(el("div", "artclaim", `Software Bill of Materials${fmt ? ` · ${escapeHtml(fmt)}` : ""} · <b>${cnt}</b> file(s)/component(s) recorded`));
  const pretty = JSON.stringify(sbom, null, 2);
  viewToggle(d, () => el("pre", "code", escapeHtml(pretty)), JSON.stringify(sbom));
}
function renderLog(text) {
  const d = $("#artdetail"); d.innerHTML = "";
  d.appendChild(el("pre", "code", escapeHtml(text || "(empty)")));
}

// Minimal, safe Markdown → HTML (escape first, then format). Handles headings, lists,
// bold/italic/inline-code, fenced code, and links.
function mdToHtml(src) {
  const esc = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
  const blocks = [];
  src = String(src).replace(/```(\w*)\n([\s\S]*?)```/g, (m, lang, code) => {
    blocks.push("<pre><code>" + esc(code) + "</code></pre>"); return " " + (blocks.length - 1) + " ";
  });
  const inline = (t) => t
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>")
    .replace(/\*([^*]+)\*/g, "<i>$1</i>")
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  let html = "", list = null;
  for (const ln of esc(src).split("\n")) {
    let m;
    if ((m = ln.match(/^(#{1,3})\s+(.*)/))) { if (list) { html += `</${list}>`; list = null; } html += `<h${m[1].length}>${inline(m[2])}</h${m[1].length}>`; continue; }
    if ((m = ln.match(/^\s*[-*]\s+(.*)/))) { if (list !== "ul") { if (list) html += `</${list}>`; html += "<ul>"; list = "ul"; } html += `<li>${inline(m[1])}</li>`; continue; }
    if ((m = ln.match(/^\s*\d+\.\s+(.*)/))) { if (list !== "ol") { if (list) html += `</${list}>`; html += "<ol>"; list = "ol"; } html += `<li>${inline(m[1])}</li>`; continue; }
    if (ln.trim() === "") { if (list) { html += `</${list}>`; list = null; } continue; }
    if (list) { html += `</${list}>`; list = null; }
    html += `<p>${inline(ln)}</p>`;
  }
  if (list) html += `</${list}>`;
  return html.replace(/ (\d+) /g, (m, i) => blocks[i]);
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
  const pt = att.statement?.predicateType || "";
  add("predicateType", /^https?:\/\//.test(pt)
    ? `<a href="${escapeHtml(pt)}" target="_blank" rel="noopener">${escapeHtml(pt)}</a>`
    : escapeHtml(pt));
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

// ---------- apps (each intent = an app; refines are its versions) ----------
async function loadApps() {
  try {
    const apps = await api("GET", "/api/apps");
    const box = $("#apps"); box.innerHTML = "";
    if (!apps.length) { box.innerHTML = '<div class="empty">No apps yet — fabricate one above.</div>'; return; }
    apps.forEach((a) => {
      const row = el("div", "approw");
      const meta = el("div", "appmeta",
        `<div class="appname">${escapeHtml(a.intent.slice(0, 70))}</div>` +
        `<div class="appsub muted">${escapeHtml(a.base)} · <span class="pill ${a.status}" style="padding:1px 7px; font-size:10px">${a.status}</span>${a.versions > 1 ? " · v" + a.versions : ""} · <span style="color:var(--accent)">open ↗</span></div>`);
      meta.style.cursor = "pointer";
      meta.title = "open this app — load its spec, code, provenance & continue (refine)";
      meta.onclick = () => openApp(a.latest_run, a.intent);
      const btns = el("div", "appbtns");
      const launch = el("button", "btn ok sm", "▶"); launch.title = "launch the app"; launch.onclick = () => launchAppById(a.latest_run, launch);
      const open = el("button", "btn ghost sm", "📁"); open.title = "open the app's folder"; open.onclick = () => openAppFolder(a.id);
      const del = el("button", "btn ghost sm", "🗑"); del.title = "delete this app"; del.onclick = () => deleteApp(a.id, a.intent);
      btns.append(launch, open, del);
      row.append(meta, btns); box.appendChild(row);
    });
  } catch (e) { /* server may be mid-write */ }
}
// Load an existing run back into the workflow view so the user can inspect it (spec,
// generated code, provenance) and continue working on it (refine → v+1).
async function openApp(rid, intent) {
  if (!rid) return;
  resetFlow();
  STATE.runId = rid; STATE.lastSeq = 0;
  if (intent) $("#intent").value = intent;   // surface the app's original intent
  try {
    const evs = await api("GET", `/api/runs/${rid}/events?since=0`);
    evs.forEach(addEvent);
    if (evs.length) STATE.lastSeq = evs[evs.length - 1].seq;
    const run = await api("GET", `/api/runs/${rid}`);
    setStatus(run.status || "running");
    if (["blocked", "accepted", "merged", "failed", "draft"].includes(run.status)) {
      await onRunDone(run);          // shows draft / product+approval and loads artifacts
    } else {
      startPolling();                // still in-flight — resume live streaming
    }
    $("#flowcard").scrollIntoView({ behavior: "smooth", block: "start" });
    toast("loaded — inspect it, or refine to continue (v→v+1)");
  } catch (e) { toast(e.message, true); }
}
// Open every artifact this run produced (source + provenance + run-state) in Finder.
async function openRunArtifacts() {
  if (!STATE.runId) return toast("open or build an app first", true);
  try { const r = await api("POST", `/api/runs/${STATE.runId}/open`); toast("opened " + r.path); }
  catch (e) { toast(e.message, true); }
}
async function launchAppById(rid, btn) {
  const old = btn.innerHTML; btn.disabled = true; btn.innerHTML = '<span class="spin"></span>';
  try {
    const r = await api("POST", `/api/runs/${rid}/launch`);
    if (r.kind === "web") { window.open(r.url, "_blank"); toast("launched → " + r.url); }
    else if (r.kind === "web-failed") { toast(r.error, true); }
    else { toast("This app is a CLI (no web server)."); }
  } catch (e) { toast(e.message, true); }
  finally { btn.disabled = false; btn.innerHTML = old; }
}
async function openAppFolder(id) {
  try { const r = await api("POST", `/api/apps/${id}/open`); toast("opened " + r.path); }
  catch (e) { toast(e.message, true); }
}
async function deleteApp(id, name) {
  if (!confirm(`Delete app "${(name || id).slice(0, 50)}"?\nThis removes all its versions, branches, and provenance.`)) return;
  try { const r = await api("DELETE", `/api/apps/${id}`); toast(`deleted (${r.deleted} run(s))`); await loadApps(); }
  catch (e) { toast(e.message, true); }
}

// ---------- util ----------
function shortDid(d) { return d && d.length > 22 ? d.slice(0, 14) + "…" + d.slice(-4) : (d || ""); }
function escapeHtml(s) { return (s || "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); }

init();
