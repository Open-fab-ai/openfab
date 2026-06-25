# Required agent-chat patch — skip the `openfab-bridge` service agent in bridge-matrix

OpenFab registers a service relay agent named **`openfab-bridge`** in agent-chat (so it can post
task messages). agent-chat's **`bridge-matrix.js`** tries to give *every* agent-chat agent a
Matrix puppet at startup — and `openfab-bridge` has no Matrix account, so it falls through to
registration, which needs `MATRIX_REG_TOKEN`. When that token is empty (common after the initial
agent-chat setup), **bridge-matrix crashes on startup** → the Matrix⇄agent-chat sync is dead →
nothing a human types in Robrix (e.g. `approve <run>`) ever reaches OpenFab.

This is a one-time edit to **the agent-chat repo** (NOT OpenFab) — it lives here only so the fix
travels with OpenFab's deployment docs. Applies to `<agent-chat>/bridge-matrix.js`, in
`MatrixBridge.start()`, the "Ensure agent accounts for all known agents" loop.

## The change

Find this loop (around the comment `// 2. Ensure agent accounts for all known agents`):

```js
    // 2. Ensure agent accounts for all known agents
    const agents = await this.fetchKnownAgentNames();
    const validAgentNames = new Set();
    const validAgentKeys = new Set();
    for (const agentName of agents) {
      validAgentNames.add(agentName);
      validAgentKeys.add(this.nameKey(agentName));
      await ensureAgentAccount(agentName);
      this.addKnownAgent(agentName);
    }
```

Replace it with (skip service relays + make a single account failure non-fatal):

```js
    // 2. Ensure agent accounts for all known agents.
    // Service relays (e.g. OpenFab's `openfab-bridge`) post in-app only and never need a Matrix
    // puppet — skip them. And a single agent's account failure must not crash the whole bridge.
    const SKIP_AGENTS = new Set(
      (process.env.MATRIX_BRIDGE_SKIP_AGENTS || 'openfab-bridge')
        .split(',').map((s) => s.trim()).filter(Boolean),
    );
    const agents = await this.fetchKnownAgentNames();
    const validAgentNames = new Set();
    const validAgentKeys = new Set();
    for (const agentName of agents) {
      if (SKIP_AGENTS.has(agentName)) {
        console.log(`Skipping Matrix puppet for service agent: ${agentName}`);
        continue;
      }
      validAgentNames.add(agentName);
      validAgentKeys.add(this.nameKey(agentName));
      try {
        await ensureAgentAccount(agentName);
        this.addKnownAgent(agentName);
      } catch (e) {
        console.warn(`Skipping agent ${agentName} (account setup failed): ${e.message}`);
      }
    }
```

## Verify

```bash
cd <agent-chat> && node --check bridge-matrix.js
# restart bridge-matrix; the log should show:  Skipping Matrix puppet for service agent: openfab-bridge
# and then  Bot syncing...   (it no longer crashes on MATRIX_REG_TOKEN)
```

## Alternative without editing code

If you'd rather not patch, set the env var the patch reads (only works if your agent-chat already
has this skip support) and ensure the pre-existing `wf_*` agents have cached tokens in
`<agent-chat>/data/matrix/bridge-state.json`:

```bash
MATRIX_BRIDGE_SKIP_AGENTS=openfab-bridge node bridge-matrix.js
```

(Upstream-friendly fix: agent-chat could mark relay-only agents as `matrixPuppet:false`.)
