"use strict";
// forgepush — publish exits for browser mode: (1) Download a zip of code + attestation
// + SBOM (zero auth, always works); (2) push to GitHub via the git-data REST API with a
// fine-grained PAT — code AND attestation land in ONE commit, so the artifact and its
// proof are born bound together. The remote repo is the durable, versioned record
// (there is no git in a browser). Gitea/Forgejo: planned; shown disabled until real.

const ForgePush = (() => {
  // ---- minimal ZIP writer (STORED entries, CRC32) — no dependencies ----
  const CRC_TABLE = (() => {
    const t = new Uint32Array(256);
    for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c >>> 0; }
    return t;
  })();
  function crc32(bytes) {
    let c = 0xffffffff;
    for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  }
  function zip(entries) { // entries: [{name, text}]
    const te = new TextEncoder(); const chunks = []; const central = []; let offset = 0;
    const u16 = (n) => new Uint8Array([n & 255, (n >> 8) & 255]);
    const u32 = (n) => new Uint8Array([n & 255, (n >> 8) & 255, (n >> 16) & 255, (n >> 24) & 255]);
    for (const e of entries) {
      const name = te.encode(e.name), data = te.encode(e.text), crc = crc32(data);
      const head = [u32(0x04034b50), u16(20), u16(0x0800), u16(0), u16(0), u16(0), u32(crc), u32(data.length), u32(data.length), u16(name.length), u16(0)];
      central.push({ name, data, crc, offset });
      for (const h of head) chunks.push(h);
      chunks.push(name, data);
      offset += head.reduce((a, h) => a + h.length, 0) + name.length + data.length;
    }
    const cdStart = offset; let cdLen = 0;
    for (const c of central) {
      const rec = [u32(0x02014b50), u16(20), u16(20), u16(0x0800), u16(0), u16(0), u16(0), u32(c.crc), u32(c.data.length), u32(c.data.length), u16(c.name.length), u16(0), u16(0), u16(0), u16(0), u32(0), u32(c.offset)];
      for (const r of rec) { chunks.push(r); cdLen += r.length; }
      chunks.push(c.name); cdLen += c.name.length;
    }
    chunks.push(u32(0x06054b50), u16(0), u16(0), u16(central.length), u16(central.length), u32(cdLen), u32(cdStart), u16(0));
    return new Blob(chunks, { type: "application/zip" });
  }

  function bundleEntries(artifacts) {
    const slug = artifacts.run.spec_ref.replace("#", "-");
    const entries = artifacts.files.map((f) => ({ name: f.path, text: f.contents }));
    if (artifacts.attestation) entries.push({ name: `provenance/${slug}.att.json`, text: JSON.stringify(artifacts.attestation, null, 2) });
    if (artifacts.sbom) entries.push({ name: `provenance/${slug}.sbom.json`, text: JSON.stringify(artifacts.sbom, null, 2) });
    entries.push({ name: "README.md", text: `# ${artifacts.run.spec_ref}\n\nFabricated by OpenFab Web (browser mode). The signed attestation in provenance/ binds\nevery file digest, the acceptance contract, and the human sign-off — verify offline with\n\`openfab verify-file --att provenance/${slug}.att.json\` or in any OpenFab Web tab.\n` });
    return entries;
  }

  function download(artifacts) {
    const blob = zip(bundleEntries(artifacts));
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${artifacts.run.spec_ref.replace("#", "-")}-openfab-bundle.zip`;
    a.click(); setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  }

  // ---- GitHub push: blobs → tree → commit → ref, ONE commit for code + attestation ----
  async function gh(token, path, method, body) {
    const r = await fetch(`https://api.github.com${path}`, {
      method: method || "GET",
      headers: { Authorization: `Bearer ${token}`, Accept: "application/vnd.github+json", "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
    });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(`GitHub ${r.status}: ${j.message || path}`);
    return j;
  }

  async function pushGitHub({ token, owner, repo, branch, message }, artifacts) {
    const base = `/repos/${owner}/${repo}`;
    const ref = await gh(token, `${base}/git/ref/heads/${branch}`).catch(async (e) => {
      if (!String(e.message).includes("404")) throw e;
      // branch missing: branch from the default branch
      const meta = await gh(token, base);
      const def = await gh(token, `${base}/git/ref/heads/${meta.default_branch}`);
      return gh(token, `${base}/git/refs`, "POST", { ref: `refs/heads/${branch}`, sha: def.object.sha });
    });
    const baseSha = ref.object.sha;
    const baseCommit = await gh(token, `${base}/git/commits/${baseSha}`);
    const entries = bundleEntries(artifacts);
    const tree = [];
    for (const e of entries) {
      const blob = await gh(token, `${base}/git/blobs`, "POST", { content: e.text, encoding: "utf-8" });
      tree.push({ path: e.name, mode: "100644", type: "blob", sha: blob.sha });
    }
    const newTree = await gh(token, `${base}/git/trees`, "POST", { base_tree: baseCommit.tree.sha, tree });
    const commit = await gh(token, `${base}/git/commits`, "POST", {
      message: message || `openfab: ${artifacts.run.spec_ref} — code + signed attestation (one commit)`,
      tree: newTree.sha, parents: [baseSha],
    });
    await gh(token, `${base}/git/refs/heads/${branch}`, "PATCH", { sha: commit.sha, force: false });
    return { sha: commit.sha, url: `https://github.com/${owner}/${repo}/commit/${commit.sha}` };
  }

  return { download, pushGitHub };
})();
