import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

process.env.OPENFAB_BRIDGE_NO_SERVER = '1';

const bridge = await import('./openfab-agentchat-bridge.mjs');

test('processed command keys survive bridge restart and prevent replay', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'openfab-bridge-cmds-'));
  const stateFile = path.join(dir, 'state.json');
  const msg = {
    id: 'm-build-1',
    source: 'matrix',
    sender_mxid: '@alice:palpo',
    source_room: '!room:palpo',
    summary: 'build blog-site',
  };
  let buildCalls = 0;

  const firstStore = bridge.createProcessedCommandStore(stateFile);
  await bridge.processMatrixCommands([msg], {
    processed: firstStore.processed,
    markProcessed: firstStore.markProcessed,
    roomProjectFn: async () => 'openfab',
    authorizeBuildFn: async () => ({ ok: true }),
    relayBuildFn: async () => {
      buildCalls += 1;
      return { ok: true, status: 200, run_id: 'run-1' };
    },
    postMessageFn: async () => {},
    logger: () => {},
  });

  const secondStore = bridge.createProcessedCommandStore(stateFile);
  await bridge.processMatrixCommands([msg], {
    processed: secondStore.processed,
    markProcessed: secondStore.markProcessed,
    roomProjectFn: async () => 'openfab',
    authorizeBuildFn: async () => ({ ok: true }),
    relayBuildFn: async () => {
      buildCalls += 1;
      return { ok: true, status: 200, run_id: 'run-2' };
    },
    postMessageFn: async () => {},
    logger: () => {},
  });

  assert.equal(buildCalls, 1);
});

test('room build command is rejected when mxid is not mapped to a maintainer', async () => {
  const processed = new Set();
  const posts = [];
  let buildCalls = 0;

  await bridge.processMatrixCommands([{
    id: 'm-build-2',
    source: 'matrix',
    sender_mxid: '@mallory:palpo',
    source_room: '!room:palpo',
    summary: 'build website',
  }], {
    processed,
    markProcessed: (key) => processed.add(key),
    roomProjectFn: async () => 'openfab',
    authorizeBuildFn: async () => ({ ok: false, reason: 'matrix user is not mapped to a maintainer' }),
    relayBuildFn: async () => {
      buildCalls += 1;
      return { ok: true, status: 200, run_id: 'run-should-not-happen' };
    },
    postMessageFn: async (_room, msg) => posts.push(msg),
    logger: () => {},
  });

  assert.equal(buildCalls, 0);
  assert.equal(processed.has('m-build-2'), true);
  assert.match(posts[0], /not authorized/i);
});

test('build authorization requires exactly one maintainer mapped to the mxid', () => {
  const maintainers = [
    { name: 'alice', mxid: '@alice:palpo' },
    { name: 'bob', mxid: '@bob:palpo' },
  ];
  assert.deepEqual(
    bridge.buildAuthorizationFromMaintainers('@alice:palpo', maintainers),
    { ok: true, maintainer: 'alice' },
  );
  assert.equal(
    bridge.buildAuthorizationFromMaintainers('@mallory:palpo', maintainers).ok,
    false,
  );
  assert.equal(
    bridge.buildAuthorizationFromMaintainers('@x:palpo', [
      { name: 'a', mxid: '@x:palpo' },
      { name: 'b', mxid: '@x:palpo' },
    ]).ok,
    false,
  );
});

test('first boot seeds historical matrix commands as processed without executing them', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'openfab-bridge-seed-'));
  const stateFile = path.join(dir, 'state.json');
  const messagesFile = path.join(dir, 'messages.json');
  const historical = {
    id: 'm-historic-build',
    source: 'matrix',
    sender_mxid: '@alice:palpo',
    source_room: '!room:palpo',
    summary: 'build blog-site',
  };
  fs.writeFileSync(messagesFile, JSON.stringify([historical]));

  // First boot: no state file yet → seed marks history processed (no execution here).
  const store = bridge.createProcessedCommandStore(stateFile);
  const seeded = bridge.seedProcessedCommands(messagesFile, stateFile, store.processed);
  assert.equal(seeded, 1);

  let buildCalls = 0;
  await bridge.processMatrixCommands([historical], {
    processed: store.processed,
    markProcessed: store.markProcessed,
    roomProjectFn: async () => 'openfab',
    authorizeBuildFn: async () => ({ ok: true }),
    relayBuildFn: async () => {
      buildCalls += 1;
      return { ok: true, status: 200, run_id: 'run-replayed' };
    },
    postMessageFn: async () => {},
    logger: () => {},
  });
  assert.equal(buildCalls, 0);

  // Second boot: state file exists → seeding never runs again.
  assert.equal(bridge.seedProcessedCommands(messagesFile, stateFile, new Set()), 0);
});

test('SSE message events feed Matrix command processing without waiting for file polling', async () => {
  const processed = new Set();
  let buildCalls = 0;

  await bridge.processSseMessageEvent(JSON.stringify({
    id: 'sse-build-1',
    source: 'matrix',
    sender_mxid: '@alice:palpo',
    source_room: '!room:palpo',
    summary: 'build realtime-site',
  }), {
    processed,
    markProcessed: (key) => processed.add(key),
    roomProjectFn: async () => 'openfab',
    authorizeBuildFn: async () => ({ ok: true }),
    relayBuildFn: async () => {
      buildCalls += 1;
      return { ok: true, status: 200, run_id: 'run-sse' };
    },
    postMessageFn: async () => {},
    logger: () => {},
  });

  assert.equal(buildCalls, 1);
  assert.equal(processed.has('sse-build-1'), true);
});
