"use strict";
// ops_browser — browser-mode backend for OpenFab Web. Implements the SAME JSON API the
// Rust server exposes (the API is the ops port), entirely client-side: the browser swarm
// generates, js: acceptance checks run for real, attestations are Ed25519-signed
// (did:key), and run state lives in IndexedDB. Routes a static page cannot honestly
// serve (host sandbox exec, git audit graph) return a clear capability error (R14 —
// never a silent stub). Versioning without git: lineage lives in spec_ref (id#vN) and
// the attestation chain; the durable record is what you Download / push to a forge.

const OpsBrowser = (() => {
  // ---- IndexedDB run store ----
  let dbp = null;
  function db() {
    if (!dbp) dbp = new Promise((res, rej) => {
      const r = indexedDB.open("openfab-web", 1);
      r.onupgradeneeded = () => r.result.createObjectStore("runs", { keyPath: "run_id" });
      r.onsuccess = () => res(r.result); r.onerror = () => rej(r.error);
    });
    return dbp;
  }
  async function putRun(rec) { const d = await db(); return new Promise((res, rej) => { const t = d.transaction("runs", "readwrite"); t.objectStore("runs").put(rec); t.oncomplete = res; t.onerror = () => rej(t.error); }); }
  async function getRun(id) { const d = await db(); return new Promise((res, rej) => { const q = d.transaction("runs").objectStore("runs").get(id); q.onsuccess = () => res(q.result || null); q.onerror = () => rej(q.error); }); }
  async function allRuns() { const d = await db(); return new Promise((res, rej) => { const q = d.transaction("runs").objectStore("runs").getAll(); q.onsuccess = () => res(q.result || []); q.onerror = () => rej(q.error); }); }
  async function delRun(id) { const d = await db(); return new Promise((res, rej) => { const t = d.transaction("runs", "readwrite"); t.objectStore("runs").delete(id); t.oncomplete = res; t.onerror = () => rej(t.error); }); }

  // ---- identities (localStorage; fab + maintainers) ----
  const IDS_KEY = "openfab_web_identities";
  function loadIds() { try { return JSON.parse(localStorage.getItem(IDS_KEY)) || {}; } catch { return {}; } }
  async function identity(name) {
    const ids = loadIds();
    if (!ids[name]) { ids[name] = await FabCrypto.createIdentity(name); localStorage.setItem(IDS_KEY, JSON.stringify(ids)); }
    return ids[name];
  }

  const LIVE = new Map(); // run_id -> in-memory record while running

  function ev(rec, icon, msg) {
    rec.events.push({ seq: rec.events.length + 1, icon, msg, t: new Date().toISOString().slice(11, 19) });
  }

  // Canonical bytes of a statement, mirroring Rust's serde attributes: empty
  // `signoffs`/`acceptance` vectors are OMITTED (skip_serializing_if = "Vec::is_empty"),
  // so browser and Rust canonicalizations are byte-identical.
  function canonicalStatement(stmt, signoffsSlice) {
    const s = JSON.parse(JSON.stringify(stmt));
    if (signoffsSlice !== undefined) s.predicate.signoffs = signoffsSlice;
    if (!s.predicate.signoffs || !s.predicate.signoffs.length) delete s.predicate.signoffs;
    if (!s.predicate.acceptance || !s.predicate.acceptance.length) delete s.predicate.acceptance;
    return FabCrypto.canonicalJson(s);
  }

  // ---- attestation (same shape + canonicalization as src/core/provenance.rs) ----
  async function buildAttestation(rec, files, checks, model, prompt) {
    const fab = await identity("fab");
    const generated = [];
    for (const [p, c] of Object.entries(files)) {
      generated.push({ path: p, lines: `1-${Math.max(1, c.split("\n").length)}`, sha256: await FabCrypto.sha256Hex(c), author: "ai" });
    }
    const bundle = generated.map((g) => `${g.path}:${g.sha256}`).sort().join("\n");
    const statement = {
      _type: "https://in-toto.io/Statement/v1",
      subject: [{ name: rec.spec_ref.replace("#", "-"), digest: { sha256: await FabCrypto.sha256Hex(bundle) } }],
      predicateType: "https://open-fab.ai/attestation/generation/v0.1",
      predicate: {
        spec_ref: rec.spec_ref,
        builder: { id: "openfab-web/0.1", base: "browser-swarm" },
        agent: { did: fab.did, base: "browser-swarm", model },
        prompt_sha256: await FabCrypto.sha256Hex(prompt),
        params: { base: "browser-swarm", mode: "browser" },
        generated, materials: [],
        acceptance_passed: checks.every((c) => c.passed),
        acceptance: (rec.spec.acceptance || []).map((a) => ({ id: a.id, check: a.check, must_pass: true, passed: !!checks.find((c) => c.id === a.id && c.passed) })),
        timestamp: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
        signoffs: [],
      },
    };
    const canonical = canonicalStatement(statement);
    return {
      payload_type: "application/vnd.in-toto+json",
      payload_sha256: await FabCrypto.sha256Hex(canonical),
      statement,
      signatures: [{ keyid: fab.did, sig: await FabCrypto.signB64(fab, canonical), algo: "ed25519", role: "fab" }],
    };
  }

  // ---- the run pipeline (async; UI polls events + status) ----
  async function startRun({ intent, gate, mode, parent }) {
    if (!(await FabCrypto.ed25519Supported())) throw new Error("this browser lacks WebCrypto Ed25519 — signing would be fake, refusing (use a current Chrome/Safari/Firefox)");
    const slug = intent.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "app";
    const version = parent ? parent.version + 1 : 1;
    const run_id = `${slug}-v${version}-${Date.now() / 1000 | 0}`;
    const rec = {
      run_id, spec_ref: `${slug}#v${version}`, version, intent, gate: gate || "solo",
      mode: mode || "release", base_name: "browser-swarm", base_runtime: "native",
      forge_name: "browser", status: "running", events: [], files: null, attestation: null,
      acceptance: [], acceptance_passed: false, accepted: false, merged: false,
      spec: null, created: new Date().toISOString(), parent_run: parent ? parent.run_id : null,
    };
    LIVE.set(run_id, rec);
    (async () => {
      const maxAttempts = 1 + retries(); // e.g. retries()=2 → up to 3 attempts
      for (let attempt = 1; attempt <= maxAttempts; attempt++) {
        try {
          // Fresh state each attempt (re-author + re-generate) so a self-correction is genuine.
          rec.files = null; rec.acceptance = []; rec.acceptance_passed = false; rec.attestation = null; rec.status = "running";
          if (attempt === 1) ev(rec, "📥", `NL intent received → "${intent.slice(0, 90)}"`);
          rec.spec = await FabEngine.authorSpec(intent);
          rec.spec_ref = `${rec.spec.id || slug}#v${version}`;
          ev(rec, "🧾", `spec authored in-browser (${rec.spec.model}) → ${(rec.spec.acceptance || []).length} acceptance criteria (js: checks)`);
          if ((rec.spec.open_questions || []).length) ev(rec, "  ", `open questions surfaced to human: ${rec.spec.open_questions.join("; ")}`);
          const gen = await FabEngine.generate(rec.spec, intent, (i, m) => ev(rec, i, m));
          rec.files = gen.files;
          ev(rec, "🧪", `running ${(rec.spec.acceptance || []).length} js: acceptance check(s) in the opaque-origin sandbox`);
          rec.acceptance = await FabEngine.runChecks(rec.spec, rec.files);
          rec.acceptance.forEach((c) => ev(rec, c.passed ? "✅" : "❌", `acceptance [${c.id}] ${c.check} → ${c.passed ? "pass" : "FAIL" + (c.detail ? ` (${c.detail})` : "")}`));
          rec.acceptance_passed = rec.acceptance.every((c) => c.passed);
          if (rec.mode === "draft") {
            rec.status = rec.acceptance_passed ? "draft" : "failed";
            if (rec.acceptance_passed) ev(rec, "⚡", "draft complete — un-attested (promote to run the trust ceremony)");
          } else if (rec.acceptance_passed) {
            rec.attestation = await buildAttestation(rec, rec.files, rec.acceptance, gen.model, gen.prompt);
            ev(rec, "🔏", `signed in-toto attestation (openfab/generation) in-browser; fab DID ${rec.attestation.signatures[0].keyid.slice(0, 24)}…; payload sha256 ${rec.attestation.payload_sha256.slice(0, 16)}`);
            rec.status = "blocked";
            ev(rec, "🛡️", "trust gate: BLOCKED — awaiting human sign-off");
          } else {
            rec.status = "failed"; // acceptance did not pass
          }
          if (rec.status !== "failed") break; // success (blocked / draft / merged-later)
          // acceptance failed — retry the whole run if attempts remain
          if (attempt < maxAttempts) { ev(rec, "🔁", `acceptance did not pass — auto-retrying (${attempt}/${maxAttempts - 1})`); continue; }
          ev(rec, "⛔", `acceptance still failing after ${maxAttempts} attempt(s) — honest failure, not a vacuous pass`);
        } catch (e) {
          if (attempt < maxAttempts) { ev(rec, "🔁", `attempt failed (${e.message}) — auto-retrying (${attempt}/${maxAttempts - 1})`); await putRun(rec); continue; }
          rec.status = "failed"; ev(rec, "✖", `run failed after ${maxAttempts} attempt(s): ${e.message}`);
        }
        break;
      }
      await putRun(rec); LIVE.delete(run_id); LIVE.set(run_id, rec); // keep latest visible
    })();
    return { run_id };
  }
  // How many auto-retries on failure (Settings; default 2).
  function retries() { const n = parseInt(localStorage.getItem("openfab_web_retries"), 10); return Number.isFinite(n) && n >= 0 ? n : 2; }

  async function loadRec(id) { return LIVE.get(id) || (await getRun(id)); }

  async function signoff(id, as) {
    const rec = await loadRec(id);
    if (!rec || !rec.attestation) throw new Error("no attested run to sign");
    if (!rec.acceptance_passed) throw new Error("acceptance failed — a human cannot sign past a failed contract");
    const signer = await identity(as || "me");
    // Sign the statement state BEFORE this sign-off is recorded (Rust add_signoff rule).
    const canonical = canonicalStatement(rec.attestation.statement);
    rec.attestation.statement.predicate.signoffs.push({ did: signer.did, name: as || "me", timestamp: new Date().toISOString().replace(/\.\d+Z$/, "Z") });
    rec.attestation.signatures.push({ keyid: signer.did, sig: await FabCrypto.signB64(signer, canonical), algo: "ed25519", role: "human-signoff" });
    const need = rec.gate === "none" ? 0 : rec.gate === "team" ? 2 : 1;
    const have = new Set(rec.attestation.statement.predicate.signoffs.map((s) => s.did)).size;
    rec.accepted = have >= need; rec.merged = rec.accepted;
    rec.status = rec.merged ? "merged" : "blocked";
    ev(rec, "✍", `sign-off by ${as || "me"} (${signer.did.slice(0, 24)}…) — ${have}/${need}`);
    if (rec.merged) ev(rec, "🔀", "gate OPEN — release accepted (publish: Download / push to a forge)");
    await putRun(rec);
    return { signer_name: as || "me", signer_did: signer.did, accepted: rec.accepted, merged: rec.merged, status: rec.status, satisfied: [], blocking: rec.merged ? [] : [`need ${need} distinct sign-off(s), have ${have}`] };
  }

  async function reproduce(id) {
    const rec = await loadRec(id);
    const att = rec && rec.attestation;
    if (!att) throw new Error("no attestation for this run");
    // Mirror Rust verify_signatures: the fab signed the statement WITHOUT signoffs (and
    // payload_sha256 pins that build-time payload); human sign-off #n signed the state
    // with the signoffs recorded before theirs.
    const buildPayload = canonicalStatement(att.statement, []);
    let signature_valid = (await FabCrypto.sha256Hex(buildPayload)) === att.payload_sha256;
    let nth = 0;
    for (const s of att.signatures) {
      if (s.role === "fab") {
        if (!(await FabCrypto.verifyB64(s.keyid, s.sig, buildPayload))) signature_valid = false;
      } else {
        const atSign = canonicalStatement(att.statement, att.statement.predicate.signoffs.slice(0, nth));
        if (!(await FabCrypto.verifyB64(s.keyid, s.sig, atSign))) signature_valid = false;
        nth++;
      }
    }
    let source_identical = true;
    for (const g of att.statement.predicate.generated) {
      const c = rec.files && rec.files[g.path];
      if (c == null || (await FabCrypto.sha256Hex(c)) !== g.sha256) source_identical = false;
    }
    const checks = await FabEngine.runChecks(rec.spec, rec.files || {});
    const all = checks.every((c) => c.passed);
    return { run_id: id, signature_valid, source_identical, all_acceptance_passed: all, reproducible: signature_valid && source_identical && all, checks: checks.map((c) => ({ check: c.check, passed: c.passed, exit_code: c.exit_code })), files_checked: att.statement.predicate.generated.length };
  }

  function buildAppHtml(rec) {
    // Inline local js/css into the entry html. The result is UNTRUSTED model output: it
    // is rendered ONLY inside a sandbox="allow-scripts" iframe (opaque origin — no
    // access to this page's localStorage/keys). Never a same-origin blob/tab (review B1).
    let html = (rec.files && rec.files["app/index.html"]) || "<h1>no app/index.html</h1>";
    html = html.replace(/<script[^>]*src=["']\.?\/?([^"']+)["'][^>]*><\/script>/g, (m, p) => {
      const c = rec.files["app/" + p.replace(/^app\//, "")]; return c != null ? `<script>\n${c}\n</script>` : m;
    }).replace(/<link[^>]*href=["']\.?\/?([^"']+\.css)["'][^>]*>/g, (m, p) => {
      const c = rec.files["app/" + p.replace(/^app\//, "")]; return c != null ? `<style>\n${c}\n</style>` : m;
    });
    return html;
  }

  async function artifacts(id) {
    const rec = await loadRec(id);
    if (!rec) throw new Error("run not found");
    const files = [];
    for (const [p, c] of Object.entries(rec.files || {})) {
      files.push({ path: p, contents: c, sha256: await FabCrypto.sha256Hex(c), author: "ai" });
    }
    const sbom = { spdxVersion: "SPDX-2.3", SPDXID: "SPDXRef-DOCUMENT", name: rec.spec_ref, creators: ["Tool: openfab-web/0.1"], files: files.map((f, i) => ({ SPDXID: `SPDXRef-File-${i}`, fileName: f.path, checksums: [{ algorithm: "SHA256", checksumValue: f.sha256 }] })) };
    return { run: { run_id: rec.run_id, spec_ref: rec.spec_ref, status: rec.status, merged: rec.merged, base_name: rec.base_name, base_runtime: rec.base_runtime, acceptance: rec.spec ? rec.spec.acceptance : [] }, spec: rec.spec, files, attestation: rec.attestation, sbom, timeline: rec.events.map((e) => `[${e.t}] ${e.icon} ${e.msg}`).join("\n") };
  }

  // ---- the route dispatcher: same contract as the Rust JSON API ----
  const CAP = (what) => { const e = new Error(`${what} needs the local OpenFab server (browser mode has no host sandbox / git) — everything else here is real: generation, checks, signing, verify`); e.status = 501; throw e; };

  async function dispatch(method, url, body) {
    const u = new URL(url, location.origin); const p = u.pathname; const q = u.searchParams;
    const seg = p.split("/").filter(Boolean); // ["api", ...]
    if (p === "/api/bases") return [{ id: "browser-swarm", display: "Browser swarm (this tab)", runtime: "native", note: "planner/coder/reviewer as direct LLM calls in this tab — configure the LLM in ⚙ Settings" }];
    if (p === "/api/forges") return [{ id: "browser", display: "In-browser (publish: Download / GitHub)", kind: "browser", live: false, note: "no local git in a browser — the pushed repo is the durable, versioned record" }];
    if (p === "/api/models") { const c = FabEngine.llmConfig(); return { author: c ? [c.model] : [], base: c ? [c.model] : [] }; }
    if (p === "/api/maintainers" && method === "GET") { const ids = loadIds(); return Object.values(ids).filter((i) => i.name !== "fab").map((i) => ({ name: i.name, did: i.did })); }
    if (p === "/api/maintainers" && method === "POST") { const i = await identity(body.name); return { name: i.name, did: i.did }; }
    if (p === "/api/reputation") {
      const runs = await allRuns(); const m = new Map();
      for (const r of runs) for (const s of ((r.attestation || {}).statement || { predicate: { signoffs: [] } }).predicate.signoffs || []) m.set(s.did, (m.get(s.did) || 0) + 1);
      const fab = (loadIds()).fab;
      return { agents: [...(fab ? [{ did: fab.did, authored: runs.filter((r) => r.attestation).length, accepted: runs.filter((r) => r.merged).length, signoffs_given: 0 }] : []), ...[...m].map(([did, n]) => ({ did, authored: 0, accepted: 0, signoffs_given: n }))] };
    }
    if (p === "/api/apps" && method === "GET") {
      const runs = (await allRuns()).sort((a, b) => (b.created || "").localeCompare(a.created || ""));
      const seen = new Map();
      for (const r of runs) { const id = r.spec_ref.split("#")[0]; if (!seen.has(id)) seen.set(id, { id, intent: r.intent, base: r.base_name, status: r.status, versions: 1, latest_run: r.run_id }); else seen.get(id).versions++; }
      return [...seen.values()];
    }
    if (seg[1] === "apps" && method === "DELETE") { const runs = await allRuns(); let n = 0; for (const r of runs) if (r.spec_ref.split("#")[0] === seg[2]) { await delRun(r.run_id); n++; } return { deleted: n }; }
    if (seg[1] === "apps" && seg[3] === "open") CAP("opening a folder");
    if (p === "/api/run" && method === "POST") return startRun({ intent: body.intent, gate: body.gate, mode: body.mode });
    if (seg[1] === "runs" && seg.length === 3 && method === "GET") { const r = await loadRec(seg[2]); if (!r) { const e = new Error("run not found"); e.status = 404; throw e; } return { run_id: r.run_id, status: r.status, spec_ref: r.spec_ref, acceptance_passed: r.acceptance_passed, accepted: r.accepted, merged: r.merged, base_name: r.base_name, gate_mode: r.gate }; }
    if (seg[1] === "runs" && seg[3] === "events") { const r = await loadRec(seg[2]); const since = Number(q.get("since") || 0); return (r ? r.events : []).filter((e) => e.seq > since); }
    if (seg[1] === "runs" && seg[3] === "artifacts") return artifacts(seg[2]);
    if (seg[1] === "runs" && seg[3] === "signoff") return signoff(seg[2], body && body.as);
    if (seg[1] === "runs" && seg[3] === "reject") { const r = await loadRec(seg[2]); r.status = "rejected"; await putRun(r); return { run_id: r.run_id, status: r.status }; }
    if (seg[1] === "runs" && seg[3] === "reproduce") return reproduce(seg[2]);
    if (seg[1] === "runs" && seg[3] === "verify") { const rep = await reproduce(seg[2]); const r = await loadRec(seg[2]); return { conformant: rep.reproducible, accepted: r.accepted, checks: rep.checks.map((c, i) => ({ id: `c${i + 1}`, passed: c.passed, detail: c.check })) }; }
    if (seg[1] === "runs" && seg[3] === "launch") { const r = await loadRec(seg[2]); return { kind: "web-sandbox", html: buildAppHtml(r) }; }
    if (seg[1] === "runs" && seg[3] === "stop") return { stopped: true }; // blob apps have no process
    if (seg[1] === "runs" && seg[3] === "feedback") { const prior = await loadRec(seg[2]); return startRun({ intent: `${prior.intent}\n\nRevision requested by the human: ${body.note}`, gate: prior.gate, mode: body.mode || prior.mode, parent: prior }); }
    if (seg[1] === "runs" && seg[3] === "promote") { const d = await loadRec(seg[2]); if (!d.acceptance_passed) throw new Error("draft failed acceptance — no vacuous promotion"); return startRun({ intent: d.intent, gate: d.gate, mode: "release", parent: { run_id: d.run_id, version: d.version - 1 } }); }
    if (seg[1] === "runs" && seg[3] === "exec") CAP("running sandbox commands");
    if (seg[1] === "runs" && seg[3] === "audit") CAP("the git audit trail");
    if (seg[1] === "runs" && seg[3] === "open") CAP("opening a folder");
    if (seg[1] === "base" && seg[3] === "launch") CAP("launching a native base");
    const e = new Error(`browser mode: no handler for ${method} ${p}`); e.status = 501; throw e;
  }

  return { dispatch, loadRec };
})();
