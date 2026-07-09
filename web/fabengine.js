"use strict";
// fabengine — the browser-mode run engine: LLM provider client (OpenAI-compatible),
// spec authoring, the in-tab "browser swarm" (coder → reviewer), and js: acceptance
// checks executed against the generated files. R14: a check either genuinely runs and
// passes, or it fails — there is no simulated pass. The LLM provider must be
// CORS-callable from a browser (e.g. OpenRouter); we surface provider errors verbatim.

const FabEngine = (() => {
  const LLM_KEY = "openfab_web_llm"; // { providerId, baseUrl, apiKey, model }

  // ---- Agent-guidance slices (openfab-agent.md) ---------------------------
  // The system prompts are NOT hardcoded here. They are the injectable slices
  // of openfab-agent.md — read LIVE on every call so editing a slice changes the
  // NEXT call of that role with no rebuild. Resolution order per slice:
  //   1. user override in localStorage (Settings → Agent guidance)
  //   2. the shipped openfab-agent.md, fetched once and parsed
  //   3. the embedded fallback below (used only if the fetch fails, e.g. offline)
  // The embedded fallbacks MIRROR the <!-- inject:* --> blocks of
  // web/openfab-agent.md (the single canonical source); keep them in sync.
  const SLICE_KEY = { shared: "openfab_web_slice_shared", spec: "openfab_web_slice_spec", coder: "openfab_web_slice_coder" };
  const SLICE_FALLBACK = {
    shared: "You are the pair-programming partner inside an OpenFab fab: the human owns intent and judgment, you own the draft. Never guess to fill a gap — surface it as an open question. Empty, skipped, or failing output is a failure, never a pass.",
    spec: "You turn a user's natural-language request into a build spec a non-technical human can read and confirm. Start with a short plain-English summary of what will be built and why (WHAT/WHY, no technology or file layout). Then give each acceptance criterion a plain-English description of what it guarantees for the user, paired with the machine check that verifies it — cover the user's ACTUAL intent (the key behaviors/elements they asked for), not incidental details; prefer a few high-signal criteria over many brittle ones, and never over-constrain the design. Do not just raise open questions: for each one, propose a recommended default answer and a one-line reason, so the human can accept or override at a glance.",
    coder: [
      "You are a senior CODER agent producing a complete, working, client-side web app (vanilla HTML/CSS/JS only). Engineering standards — follow them, in priority order:",
      "• Correctness & robustness first: pass every acceptance check; handle empty/invalid/boundary input; no console errors; no external network/CDN dependencies.",
      "• KISS & simplicity: the simplest design that fully meets the spec; no speculative features or frameworks (YAGNI).",
      "• Single responsibility & modularity: small, well-named functions each doing one thing; separate structure/style/behavior.",
      "• DRY: never duplicate logic or markup — factor shared behavior into one place; no copy-paste blocks; any value used twice lives once.",
      "• Readability: clear names (functions are verbs, types are nouns), no magic numbers, brief comments only where intent isn't obvious; no dead or commented-out code.",
      "• Accessibility & UX basics: labels for inputs, keyboard-usable, sensible defaults.",
      "Produce the smallest set of files that works; include every file the app references.",
    ].join("\n"),
  };
  // Defaults parsed from the shipped openfab-agent.md (populated by loadShippedSlices).
  const shippedSlices = { shared: null, spec: null, coder: null };
  function parseInjectBlock(md, name) {
    const m = md.match(new RegExp("<!--\\s*inject:" + name + "\\s*-->([\\s\\S]*?)<!--\\s*/inject:" + name + "\\s*-->"));
    return m ? m[1].trim() : null;
  }
  let shippedPromise = null;
  function loadShippedSlices() {
    // Fetch + parse once; on failure the embedded fallbacks are used. Not fatal:
    // guidance drives quality, and a stale/missing file must not block a run.
    if (!shippedPromise) {
      shippedPromise = fetch("openfab-agent.md")
        .then((r) => (r.ok ? r.text() : ""))
        .then((md) => { for (const k of Object.keys(shippedSlices)) shippedSlices[k] = parseInjectBlock(md, k); })
        .catch(() => { /* offline / server mode: embedded fallbacks apply */ });
    }
    return shippedPromise;
  }
  function slice(name) {
    const override = localStorage.getItem(SLICE_KEY[name]);
    if (override != null && override.trim()) return override;
    return shippedSlices[name] || SLICE_FALLBACK[name];
  }
  function sliceDefault(name) { return shippedSlices[name] || SLICE_FALLBACK[name]; } // for the Settings editor
  function setSlice(name, text) {
    if (text == null || text === sliceDefault(name)) localStorage.removeItem(SLICE_KEY[name]);
    else localStorage.setItem(SLICE_KEY[name], text);
  }
  function exportSlices() {
    return JSON.stringify({ shared: slice("shared"), spec: slice("spec"), coder: slice("coder") }, null, 2);
  }
  function importSlices(json) {
    const o = JSON.parse(json); // throws on bad input — surfaced to the caller (Settings)
    for (const k of Object.keys(SLICE_KEY)) if (typeof o[k] === "string") setSlice(k, o[k]);
  }
  // Ask the LLM to improve one guidance slice. Returns the improved text (the
  // human reviews + saves it — it is never applied silently). `role` labels what
  // the slice governs so the model keeps it scoped and concise.
  const SLICE_ROLE = {
    shared: "the shared preamble injected into EVERY LLM call (keep it very short — it is paid on every call)",
    spec: "the spec-author's guidance (turning intent into machine-checkable acceptance criteria; WHAT/WHY, never HOW)",
    coder: "the coder's guidance (engineering standards for generating a client-side web app: KISS, DRY, SRP, readability, robustness)",
  };
  async function improveSlice(name, current) {
    const sys = "You are a prompt engineer improving one section of a system prompt. Return ONLY the improved section text — no preamble, no markdown fences, no commentary. Keep it concise and high-signal; preserve the original intent and scope; do not add rules the section wasn't about.";
    const usr = `This section is ${SLICE_ROLE[name] || name}.\n\nImprove it (clarity, specificity, concision). Return only the replacement text.\n\nCURRENT:\n${current}`;
    const out = await chat(sys, usr, roleModel("spec"));
    return (out.text || "").trim();
  }

  // Presets. `browser` = CORS verified/known; Ollama Cloud tested 2026-07: no CORS.
  const PROVIDERS = [
    { id: "openrouter", name: "OpenRouter (browser-ready ✓)", baseUrl: "https://openrouter.ai/api/v1", browser: true },
    { id: "anthropic", name: "Anthropic Claude (browser-ready ✓)", baseUrl: "https://api.anthropic.com/v1", browser: true },
    { id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com/v1", browser: true },
    { id: "ollama-cloud", name: "Ollama Cloud (no browser CORS yet ⚠)", baseUrl: "https://ollama.com/v1", browser: false },
    { id: "dashscope", name: "DashScope / Qwen", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", browser: false },
    { id: "nvidia", name: "NVIDIA NIM", baseUrl: "https://integrate.api.nvidia.com/v1", browser: false },
    { id: "custom", name: "Custom (OpenAI-compatible)", baseUrl: "", browser: true },
  ];

  // A curated, cross-brand shortlist per provider — offered as an editable dropdown
  // (the field stays free-text, so any model id still works). OpenRouter slugs verified live.
  const SUGGESTED = {
    openrouter: [
      "z-ai/glm-4.7-flash", "z-ai/glm-5.2", "qwen/qwen3-coder",
      "anthropic/claude-sonnet-4.5", "openai/gpt-5.1", "google/gemini-2.5-flash",
      "google/gemini-2.5-pro", "deepseek/deepseek-chat-v3.1", "moonshotai/kimi-k2",
      "meta-llama/llama-4-maverick",
    ],
    openai: ["gpt-5.1", "gpt-4.1", "o4-mini"],
    anthropic: ["claude-sonnet-4-5", "claude-opus-4-1"],
    "ollama-cloud": ["glm-5.2:cloud", "qwen3-coder:480b-cloud", "deepseek-v3.1:cloud"],
    dashscope: ["qwen3-coder", "qwen-max", "qwen-plus"],
    nvidia: ["deepseek-ai/deepseek-r1", "qwen/qwen3-coder"],
    custom: [],
  };
  function suggestedModels(providerId) { return SUGGESTED[providerId] || []; }

  function llmConfig() {
    try { return JSON.parse(localStorage.getItem(LLM_KEY)) || null; } catch { return null; }
  }
  function saveLlmConfig(cfg) { localStorage.setItem(LLM_KEY, JSON.stringify(cfg)); }

  // Test an (unsaved) config with a tiny completion — returns {ok, model} or {ok:false, error}.
  async function probe(cfg) {
    if (!cfg.baseUrl || !cfg.model) return { ok: false, error: "set a Base URL and Model first" };
    const url = cfg.baseUrl.replace(/\/+$/, "") + "/chat/completions";
    const headers = { "Content-Type": "application/json" };
    if (cfg.apiKey) headers.Authorization = `Bearer ${cfg.apiKey}`;
    if (cfg.providerId === "anthropic") headers["anthropic-dangerous-direct-browser-access"] = "true";
    try {
      const r = await fetch(url, { method: "POST", headers, body: JSON.stringify({ model: cfg.model, max_tokens: 5, messages: [{ role: "user", content: "ping" }] }) });
      if (!r.ok) return { ok: false, error: `HTTP ${r.status}: ${(await r.text().catch(() => "")).slice(0, 120)}` };
      const j = await r.json();
      return { ok: true, model: j.model || cfg.model };
    } catch (e) { return { ok: false, error: `${e.message} — the provider may not allow browser (CORS) calls` }; }
  }

  // Optional per-role models (Advanced): fall back to the primary model when unset.
  function roleModel(role) {
    const c = llmConfig() || {};
    return (role === "spec" ? c.specModel : role === "coder" ? c.coderModel : "") || c.model;
  }
  // Fetch the full OpenRouter catalogue (no auth needed) → sorted ids, for "load all models".
  async function loadOpenRouterModels() {
    const r = await fetch("https://openrouter.ai/api/v1/models");
    if (!r.ok) throw new Error(`OpenRouter models HTTP ${r.status}`);
    return (await r.json()).data.map((m) => m.id).sort();
  }

  async function chat(system, user, modelOverride) {
    const cfg = llmConfig();
    if (!cfg || !cfg.baseUrl || !cfg.model) throw new Error("no LLM configured — open ⚙ Settings and set a provider, key and model");
    const url = cfg.baseUrl.replace(/\/+$/, "") + "/chat/completions";
    const headers = { "Content-Type": "application/json" };
    if (cfg.apiKey) headers.Authorization = `Bearer ${cfg.apiKey}`;
    if (cfg.providerId === "anthropic") headers["anthropic-dangerous-direct-browser-access"] = "true";
    const r = await fetch(url, {
      method: "POST", headers,
      body: JSON.stringify({ model: modelOverride || cfg.model, temperature: 0, stream: false, max_tokens: 6000,
        messages: [{ role: "system", content: system }, { role: "user", content: user }] }),
    });
    if (!r.ok) throw new Error(`LLM ${r.status}: ${(await r.text().catch(() => "")).slice(0, 180)}`);
    const j = await r.json();
    const msg = j?.choices?.[0]?.message || {};
    // Reasoning models (e.g. qwen3) sometimes return an empty `content` with the real
    // output in a reasoning/thinking field — fall back before failing.
    const content = msg.content || msg.reasoning_content || msg.reasoning || msg.thinking;
    if (!content) throw new Error("LLM response missing choices[0].message.content");
    return { text: content, model: j.model || cfg.model };
  }

  function parseJson(text) {
    let t = String(text).trim();
    // Reasoning models sometimes emit <think>…</think> before the answer — drop it.
    t = t.replace(/<think>[\s\S]*?<\/think>/gi, "").trim();
    const fence = t.match(/```(?:json)?\s*([\s\S]*?)```/i);
    if (fence) t = fence[1].trim();
    try { return JSON.parse(t); } catch (_) { /* fall through */ }
    const i = t.indexOf("{"), j = t.lastIndexOf("}");
    if (i >= 0 && j > i) { try { return JSON.parse(t.slice(i, j + 1)); } catch (_) { /* fall through */ } }
    throw new Error("could not parse JSON from the model reply");
  }

  // Parse, or ask the model once to re-emit as strict JSON (models occasionally wrap or
  // truncate). A second failure throws — we never fabricate a result (R14).
  async function chatJson(system, user, modelOverride) {
    const first = await chat(system, user, modelOverride);
    try { return { obj: parseJson(first.text), model: first.model }; }
    catch (_) {
      const retry = await chat("Return ONLY the corrected JSON object — no prose, no code fences, no <think> blocks.",
        `The following was supposed to be a single JSON object but did not parse. Re-emit it as strict, complete JSON:\n\n${first.text.slice(0, 12000)}`, modelOverride);
      return { obj: parseJson(retry.text), model: first.model };
    }
  }

  // ---- spec authoring (browser targets + js: checks only — honestly runnable here) ----
  // One source (R3) for the spec JSON shape + rules, shared by authorSpec and reauthorSpec.
  // The spec is human-facing: a plain-English `summary` and per-criterion `desc` the user can
  // read/edit in the spec-review pause; the `js:` check is the machine encoding behind each.
  const SPEC_SHAPE = `{"id":"<kebab-slug>","language":"html/js","target_dir":"app",
 "summary":"<one short paragraph, plain English: what the app does and for whom>",
 "acceptance":[{"id":"a1-<slug>","desc":"<plain-English: what this guarantees for the user>","check":"js:<expression>"}, ...],
 "assumptions":["..."],
 "open_questions":["<question> — Suggested: <recommended answer> (<one-line reason>)"]}`;
  const SPEC_RULES = `Rules:
- Pure client-side HTML/CSS/JS under app/ (entry app/index.html). No servers, no build tools.
- Each "desc" is plain English a non-technical user understands. Each "check" is a JavaScript EXPRESSION
  prefixed "js:", evaluated with a variable \`files\` (a map of path -> contents), returning true when satisfied.
  Examples: "js:!!files['app/index.html']" · "js:files['app/index.html'].includes('id=\\"add-btn\\"')"
- 2 to 4 criteria that GENUINELY verify the request. Assert the smallest stable token (an id= or function
  name), never a whole tag with attributes; never over-constrain the design.
- Every open_question MUST include a "Suggested:" answer and a one-line reason in parentheses.`;
  async function authorSpec(intent) {
    await loadShippedSlices();
    // System prompt = shared slice + spec slice, read live from openfab-agent.md /
    // Settings (see slice()). The BROWSER-ONLY target + JSON shape are call plumbing.
    const sys = `${slice("shared")}\n\n${slice("spec")}\n\nTarget: a BROWSER-ONLY web app (pure client-side HTML/CSS/JS, no servers, no build).`;
    const usr = `Respond with ONLY a JSON object (no prose):
${SPEC_SHAPE}

${SPEC_RULES}
USER REQUEST:
${intent}`;
    const out = await chat(sys, usr, roleModel("spec"));
    const a = parseJson(out.text);
    a.model = out.model;
    return a;
  }

  // Re-author a spec from human feedback (used by the spec-review pause). Same
  // output shape as authorSpec; the model sees the prior spec + the human's ask.
  async function reauthorSpec(intent, prevSpec, feedback) {
    await loadShippedSlices();
    const sys = `${slice("shared")}\n\n${slice("spec")}\n\nTarget: a BROWSER-ONLY web app (pure client-side HTML/CSS/JS, no servers, no build).`;
    const usr = `Revise the spec below per the human's feedback. Respond with ONLY the updated JSON object, same shape:
${SPEC_SHAPE}

${SPEC_RULES}
ORIGINAL USER REQUEST:
${intent}

CURRENT SPEC:
${JSON.stringify(prevSpec, null, 2)}

HUMAN FEEDBACK (apply this):
${feedback || "(no free-text feedback — tighten/clarify the spec while preserving intent)"}`;
    const out = await chat(sys, usr, roleModel("spec"));
    const a = parseJson(out.text);
    a.model = out.model;
    return a;
  }

  // ---- the browser swarm: coder → reviewer (both real LLM calls in this tab) ----
  const FILES_SHAPE = `Respond with ONLY one JSON object, no prose:
{"files": {"app/<relpath>": "<full file contents>", ...}, "notes": "<one line>"}`;

  function taskBlock(spec, intent) {
    const checks = (spec.acceptance || []).map((c, i) => `  ${i + 1}. [${c.id}] ${c.check}`).join("\n");
    return `TASK: ${intent}\nTARGET: pure client-side web app under app/ (entry app/index.html; inline or local js/css only).\nACCEPTANCE (js: expressions over a files map — your files MUST make each return true):\n${checks}`;
  }

  // Coder system prompt = shared slice + coder slice, read LIVE per call (see slice()).
  function coderSys() { return `${slice("shared")}\n\n${slice("coder")}`; }
  function normalizeFiles(files) {
    const norm = {};
    for (const [p, c] of Object.entries(files || {})) norm[p.startsWith("app/") ? p : "app/" + p.replace(/^\/+/, "")] = String(c);
    return norm;
  }
  function dumpFiles(files) {
    return Object.entries(files).map(([p, c]) => `--- ${p} ---\n${c}`).join("\n\n").slice(0, 24000);
  }

  // coder → run the REAL acceptance checks → revise ONLY on a real failure. This replaces
  // a separate "reviewer" LLM call (which only *guessed* whether the checks pass) with the
  // deterministic checks themselves — the acceptance contract IS the reviewer. Common apps
  // pass on the first try, so most runs make just ONE code-gen call (faster + more honest:
  // the revise is grounded in the actual failing checks, not a model's opinion).
  async function generate(spec, intent, onEvent) {
    await loadShippedSlices();
    const sys = coderSys(); // live slice; frozen for both passes of this one run
    const tb = taskBlock(spec, intent);
    onEvent("🤖", "coder: generating the app (in-tab LLM call)");
    const coder = await chatJson(sys,
      `${tb}\n\nEvery path starts with "app/". Include every file the app references. Use the EXACT ids/tokens the checks assert.\n${FILES_SHAPE}`, roleModel("coder"));
    let files = normalizeFiles(coder.obj.files);

    onEvent("🧪", "checking against the acceptance contract");
    const failed = (await runChecks(spec, files)).filter((c) => !c.passed);
    if (failed.length) {
      onEvent("🔧", `coder: ${failed.length} acceptance check(s) failed — one revision pass`);
      const rev = await chatJson(sys,
        `${tb}\n\nCURRENT FILES:\n${dumpFiles(files)}\n\nThese acceptance checks FAILED — change the files so EVERY one passes verbatim:\n` +
        failed.map((c, i) => `  ${i + 1}. [${c.id}] ${c.check}${c.detail ? " → " + c.detail : ""}`).join("\n") +
        `\n\nReturn the COMPLETE corrected file set. ${FILES_SHAPE}`, roleModel("coder"));
      if (rev.obj.files && Object.keys(rev.obj.files).length) files = normalizeFiles(rev.obj.files);
    }
    return { files, model: coder.model, prompt: tb };
  }

  // ---- acceptance: js: expressions run for real, but NEVER in this page's realm ----
  // The checks are LLM-authored (untrusted). They execute inside a sandboxed iframe
  // WITHOUT allow-same-origin: an opaque (null) origin with no access to this page's
  // localStorage/IndexedDB (where the user's LLM key and signing keys live). The
  // evaluator only ever returns {passed, detail} booleans back via postMessage.
  let evalSeq = 0;
  const EVAL_SRC = `<script>
    window.onmessage = function (e) {
      var d = e.data || {};
      var passed = false, detail = "";
      try { passed = !!Function("files", '"use strict"; return (' + d.expr + ');')(d.files); }
      catch (err) { detail = String((err && err.message) || err); }
      e.source.postMessage({ __fabcheck: d.id, passed: passed, detail: detail }, "*");
    };
  <\/script>`;
  // Each check runs in its OWN fresh opaque-origin iframe, discarded after — so a
  // malicious js: check can never persist state or forge the result of a LATER check
  // (it shares no realm with the others). The parent trusts a reply only when both the
  // sender window identity (e.source) and the per-call id match (R14).
  function runOneCheck(expr, files) {
    return new Promise((res) => {
      const id = "c" + (++evalSeq);
      const frame = document.createElement("iframe");
      frame.setAttribute("sandbox", "allow-scripts"); // opaque origin: no storage/cookies
      frame.style.display = "none";
      frame.srcdoc = EVAL_SRC;
      const done = (r) => { clearTimeout(timer); window.removeEventListener("message", on); frame.remove(); res(r); };
      const timer = setTimeout(() => done({ passed: false, detail: "check timed out (3s)" }), 3000);
      function on(e) {
        if (e.source !== frame.contentWindow || !e.data || e.data.__fabcheck !== id) return;
        done({ passed: !!e.data.passed, detail: e.data.detail || "" });
      }
      window.addEventListener("message", on);
      frame.onload = () => frame.contentWindow.postMessage({ id, expr, files }, "*");
      document.body.appendChild(frame);
    });
  }
  async function runChecks(spec, files) {
    const out = [];
    for (const a of spec.acceptance || []) {
      const expr = String(a.check || "").replace(/^js:/, "");
      const r = await runOneCheck(expr, files);
      out.push({ id: a.id, check: a.check, passed: r.passed, exit_code: r.passed ? 0 : 1, detail: r.detail });
    }
    return out;
  }

  return { PROVIDERS, suggestedModels, loadOpenRouterModels, llmConfig, saveLlmConfig, probe, chat, authorSpec, reauthorSpec, generate, runChecks,
    loadShippedSlices, slice, sliceDefault, setSlice, exportSlices, importSlices, improveSlice };
})();
