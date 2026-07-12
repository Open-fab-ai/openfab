# FSF-0B1 Agent-Chat Supervised Services Candidate Evidence

- Evidence date: 2026-07-12
- Status: candidate only; no human sign-off and no acceptance transition
- Source repository: `/Users/zhangalex/Work/Projects/consult/agent-chat`
- Source revision: `bf8536569afc204bfa07c6274329ac1fae5d41b4`
- Source branch: `feat/matrix-agent-capabilities`
- Worktree state: dirty, 19 changed or untracked entries
- Exact B1 file-manifest digest: `2603266644a4d00817d01eefdce19ab16894b53463ceffb6460dfe3afd8a875d`
- Built image id: `sha256:715c527072d954040fac4d4b2b61d9ed4392b465d2ed6376197ada63baf953a6`
- Built image user: `node` (uid 1000 in the inspected image)

## Candidate Result

The B1 candidate defines one validated local profile and one four-service team
Compose profile for `backend`, `dashboard`, `bridge`, and `relay`. Local service
children are lease-bound to the supervisor, concurrent starts are serialized,
status/doctor are bounded, PID operations bind to process-start identities, and
the dashboard roster remains registry-owned. Compose uses a crash-safe `flock`
plus a managed owner marker; foreign, malformed, or otherwise unmanaged bridge
owner locks fail closed.

The five exact task selectors passed. `services_start_all_healthy` launched all
four production entry scripts in an isolated runtime with random ports and a
local non-routing Matrix socket. It did not use Claude, Codex, Palpo, or a
remote Matrix service. Matrix routing correctness remains B2 scope.

## Verification

Scoped B1 command:

```bash
npx vitest run \
  tests/service-profile.test.js \
  tests/local-service-supervisor.test.js \
  tests/agentchat-services-cli.test.js \
  tests/backend-lifecycle.test.js \
  tests/server-dashboard-boundary.test.js \
  tests/services-team-compose.test.js \
  tests/bridge-container-owner.test.js \
  tests/fsf0-b1-services.test.js \
  --no-file-parallelism --maxWorkers=1
```

Result: **8 files, 93 tests passed, 0 failed**. The exact selector file reports
5 passed and no skipped selector. Process inspection after the run found no B1
test supervisor or service child.

Additional checks:

```text
docker compose -f services/services-team.compose.yml config --quiet  exit 0
node --check (all new .mjs files)                              exit 0
sh -n services/run-bridge-container.sh                        exit 0
git diff --check                                               exit 0
docker build -f services/Dockerfile ...                       exit 0
agent-spec parse + lint --min-score 0.7                       quality 100%
```

Two independent Codex review passes first rejected orphaning, start races,
Compose port drift, bridge lock recovery, status bounds, PID reuse, weak
selector evidence, cross-namespace ownership, and malformed lock handling.
Regression tests were added for each reproduced issue. The final read-only
review returned **accept** with no Critical, High, or Medium findings.

## Full-Suite Disclosure

The final repository-wide run was not green: **717 passed, 7 failed, 724 total**.
Four failures are the existing macOS/BSD `sed` incompatibility in install script
tests, one is the existing push-relay delivery expectation, and one is the
existing verify-CI timeout command/exit-code mismatch. A seventh
`server-delivery` case failed once with `ECONNRESET` and passed immediately when
rerun alone. None of these files are in the B1 allowed-change set, and the six
stable failures match the pre-B1 full-suite result. This disclosure prevents the
scoped result from being treated as a clean repository release gate.

## Exact-Byte Boundary

The manifest digest covers these 24 files:

```text
services/Dockerfile
services/Dockerfile.dockerignore
services/FSF0-B1-IMPLEMENTATION-PLAN.md
services/README.md
services/agentchat-services.mjs
services/prepare-bridge-container.mjs
services/run-bridge-container.sh
services/services-local.json
services/services-team.compose.yml
src/bridge-container-owner.mjs
src/local-service-supervisor.mjs
src/process-identity.mjs
src/service-profile.mjs
src/supervised-service-child.mjs
tests/backend-lifecycle.test.js
tests/server-dashboard-boundary.test.js
tests/agentchat-services-cli.test.js
tests/bridge-container-owner.test.js
tests/fixtures/service-child.mjs
tests/fixtures/services-local-test.json
tests/fsf0-b1-services.test.js
tests/local-service-supervisor.test.js
tests/service-profile.test.js
tests/services-team-compose.test.js
```

The worktree is dirty and this candidate is not committed. Acceptance requires
an immutable revision or signed artifact for these exact bytes plus accountable
human sign-off.
