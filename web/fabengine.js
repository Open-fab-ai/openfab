"use strict";
// fabengine — the browser-mode run engine: LLM provider client (OpenAI-compatible),
// spec authoring, the in-tab "browser swarm" (coder → reviewer), and js: acceptance
// checks executed against the generated files. R14: a check either genuinely runs and
// passes, or it fails — there is no simulated pass. The LLM provider must be
// CORS-callable from a browser (e.g. OpenRouter); we surface provider errors verbatim.

const FabEngine = (() => {
  const LLM_KEY = "openfab_web_llm"; // { providerId, baseUrl, apiKey, model }

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

  function llmConfig() {
    try { return JSON.parse(localStorage.getItem(LLM_KEY)) || null; } catch { return null; }
  }
  function saveLlmConfig(cfg) { localStorage.setItem(LLM_KEY, JSON.stringify(cfg)); }

  async function chat(system, user) {
    const cfg = llmConfig();
    if (!cfg || !cfg.baseUrl || !cfg.model) throw new Error("no LLM configured — open ⚙ Settings and set a provider, key and model");
    const url = cfg.baseUrl.replace(/\/+$/, "") + "/chat/completions";
    const headers = { "Content-Type": "application/json" };
    if (cfg.apiKey) headers.Authorization = `Bearer ${cfg.apiKey}`;
    if (cfg.providerId === "anthropic") headers["anthropic-dangerous-direct-browser-access"] = "true";
    const r = await fetch(url, {
      method: "POST", headers,
      body: JSON.stringify({ model: cfg.model, temperature: 0, stream: false, max_tokens: 6000,
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
  async function chatJson(system, user) {
    const first = await chat(system, user);
    try { return { obj: parseJson(first.text), model: first.model }; }
    catch (_) {
      const retry = await chat("Return ONLY the corrected JSON object — no prose, no code fences, no <think> blocks.",
        `The following was supposed to be a single JSON object but did not parse. Re-emit it as strict, complete JSON:\n\n${first.text.slice(0, 12000)}`);
      return { obj: parseJson(retry.text), model: first.model };
    }
  }

  // ---- spec authoring (browser targets + js: checks only — honestly runnable here) ----
  async function authorSpec(intent) {
    const sys = "You turn a user's natural-language request into a machine-checkable build spec for a BROWSER-ONLY web app.";
    const usr = `Respond with ONLY a JSON object (no prose):
{"id":"<kebab-slug>","language":"html/js","target_dir":"app",
 "acceptance":[{"id":"a1-<slug>","check":"js:<expression>"}, ...],
 "assumptions":["..."],"open_questions":["..."]}

Rules for acceptance (the contract the built app is verified against):
- The app must be pure client-side HTML/CSS/JS under app/ (entry app/index.html). No servers, no build tools.
- Each check is a JavaScript EXPRESSION prefixed "js:", evaluated with a variable \`files\`
  (a map of path -> file contents as strings). It must return true when satisfied.
  Examples: "js:!!files['app/index.html']" · "js:files['app/index.html'].includes('id=\\"add-btn\\"')"
  · "js:files['app/app.js'].includes('localStorage')"
- 2 to 4 checks that GENUINELY verify the request. Assert the smallest stable token
  (an id= or function name), never a whole tag with attributes.
USER REQUEST:
${intent}`;
    const out = await chat(sys, usr);
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

  const CODER_SYS = "You are a senior CODER agent. Produce a complete, working, client-side web app. Use only vanilla HTML/CSS/JS.";
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
    const tb = taskBlock(spec, intent);
    onEvent("🤖", "coder: generating the app (in-tab LLM call)");
    const coder = await chatJson(CODER_SYS,
      `${tb}\n\nEvery path starts with "app/". Include every file the app references. Use the EXACT ids/tokens the checks assert.\n${FILES_SHAPE}`);
    let files = normalizeFiles(coder.obj.files);

    onEvent("🧪", "checking against the acceptance contract");
    const failed = (await runChecks(spec, files)).filter((c) => !c.passed);
    if (failed.length) {
      onEvent("🔧", `coder: ${failed.length} acceptance check(s) failed — one revision pass`);
      const rev = await chatJson(CODER_SYS,
        `${tb}\n\nCURRENT FILES:\n${dumpFiles(files)}\n\nThese acceptance checks FAILED — change the files so EVERY one passes verbatim:\n` +
        failed.map((c, i) => `  ${i + 1}. [${c.id}] ${c.check}${c.detail ? " → " + c.detail : ""}`).join("\n") +
        `\n\nReturn the COMPLETE corrected file set. ${FILES_SHAPE}`);
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

  return { PROVIDERS, llmConfig, saveLlmConfig, chat, authorSpec, generate, runChecks };
})();
