# Enterprise Roadmap Revision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Revise the enterprise software-factory and agentd roadmaps into one consistent cross-system plan with Specify as the required enterprise Project Authority.

**Architecture:** The factory roadmap owns cross-system roles, artifact flow, phase gates, and enterprise acceptance without assigning agentd spec numbers. The agentd roadmap owns one executable implementation sequence aligned to the current P264-P271 working-tree candidate baseline; the old N-series becomes a non-executable historical mapping.

**Tech Stack:** Markdown, agent-spec task contracts, Rust project boundaries, Matrix/Palpo, Robrix, ARC, agentd, OpenFab.

## Global Constraints

- Modify documentation only; do not change Rust, JavaScript, schemas, policies, or configuration.
- Specify is required in enterprise mode through `ProjectAuthorityPort`; standalone mode uses `LocalProjectAuthority` explicitly.
- Specify-web UX, implementation language, storage technology, and deployment topology remain deferred.
- ARC is a requirements-to-spec compiler only.
- agentd owns durable execution control and workers; OpenFab owns certification; Matrix/Robrix owns interaction and transport.
- Preserve OpenFab's base-agnostic core and optional `gate=none` delivery path.
- Do not claim the untracked P264-P271 candidate specs are merged or released.
- Do not assign speculative agentd task IDs beyond the current working sequence.
- Do not commit; leave both roadmap revisions for human review.

---

### Task 1: Rewrite the Cross-System Factory Roadmap

**Files:**
- Modify: `docs/ROADMAP-enterprise-software-factory.md`

**Interfaces:**
- Consumes: OpenFab PRD, agentd P264 ownership boundary, the confirmed Specify decision, and the Phase 1 plan.
- Produces: the authoritative cross-system role matrix, artifact flow, phase sequence, security baseline, and measurable acceptance gates.

- [x] **Step 1: Replace ambiguous system roles with authoritative ownership**

Define Specify Project Authority, ARC compiler, Robrix cockpit, Palpo/Matrix transport, agentd execution control/worker fleet, OpenFab certification/Skill Hub, and transitional agent-chat.

- [x] **Step 2: Define immutable cross-system contracts**

Specify versioned requirement/spec snapshots, execution evidence envelopes, certification state, ownership metadata, content hashes, policy versions, and failure behavior.

- [x] **Step 3: Replace the old phase sequence**

Use factory-prefixed phases: reliability baseline, authority/contracts, execution security/control plane, durable leases/workers, Matrix/Robrix cutover, OpenFab evidence/Skill Hub, native runtime/final cutover, and Kubernetes/multi-region scale.

- [x] **Step 4: Add measurable acceptance profiles**

Define pilot and enterprise gates for restart recovery, duplicate suppression, stale-lease rejection, tenant isolation, evidence completeness, availability, RPO/RTO, and workload targets.

### Task 2: Rewrite the agentd Execution Roadmap

**Files:**
- Modify: `/Users/zhangalex/Work/Projects/AI/agentd-agent-chat-replacement/docs/plans/2026-07-09-agentd-native-runtime-roadmap.md`

**Interfaces:**
- Consumes: the factory role matrix, agentd P264 ownership boundary, and current working-tree P264-P271 candidate specs.
- Produces: one active agentd execution sequence with completed/current/future status and no duplicate task IDs.

- [x] **Step 1: Replace the stale baseline and ownership text**

Record P264-P271 as the current unmerged candidate baseline, Specify as enterprise Project Authority, and agentd as execution control plane plus workers.

- [x] **Step 2: Collapse E-series and N-series into one dependency order**

Keep one active `AD-E` sequence: ownership/identity, control-plane data and APIs, sandbox/auth, lease semantics, worker fleet, Matrix gateway, OpenFab evidence integration, native runtime, and cutover.

- [x] **Step 3: Remove speculative and colliding task IDs**

Reference P264-P271 only as unmerged working-tree candidates. Describe future
contract groups without reserving IDs; assign IDs only when an agent-spec task is
created.

- [x] **Step 4: Add migration and failure semantics**

Define observe, shadow-read-only, canary, per-project cutover, drain, retire, rollback triggers, Specify fail-closed behavior, worker fencing, and independent OpenFab verification.

- [x] **Step 5: Preserve historical intent without executable ambiguity**

Move the useful native-runtime concepts into a non-numbered historical mapping and make clear that it is not a second roadmap.

### Task 3: Cross-Document Verification

**Files:**
- Verify: `docs/ROADMAP-enterprise-software-factory.md`
- Verify: `/Users/zhangalex/Work/Projects/AI/agentd-agent-chat-replacement/docs/plans/2026-07-09-agentd-native-runtime-roadmap.md`

**Interfaces:**
- Consumes: both revised documents.
- Produces: evidence that roles, phases, terms, and task IDs are internally consistent.

- [x] **Step 1: Scan for stale terminology and duplicate task IDs**

Run:

```bash
rg -n 'p223|Phase N[0-9]|Specify/OpenFab control plane|ARC (executes|runs|owns execution)' docs/ROADMAP-enterprise-software-factory.md /Users/zhangalex/Work/Projects/AI/agentd-agent-chat-replacement/docs/plans/2026-07-09-agentd-native-runtime-roadmap.md
```

Expected: no matches (exit code 1). Note: an earlier `ARC.*execution` pattern
falsely matched the legitimate ARC "must not own ... runtime execution" boundary
row; the tightened pattern only catches claims that ARC executes work.

- [x] **Step 2: Verify required ownership terms**

Run:

```bash
rg -n 'SpecifyProjectAuthority|AgentdControlPlane|OpenFabCertificationAuthority|LocalProjectAuthority|ProjectAuthorityPort' docs/ROADMAP-enterprise-software-factory.md /Users/zhangalex/Work/Projects/AI/agentd-agent-chat-replacement/docs/plans/2026-07-09-agentd-native-runtime-roadmap.md
```

Expected: each role appears with one authoritative state boundary.

- [x] **Step 3: Run Markdown and diff checks**

Run:

```bash
git diff --check -- docs/ROADMAP-enterprise-software-factory.md docs/superpowers/plans/2026-07-11-enterprise-roadmap-revision.md
git -C /Users/zhangalex/Work/Projects/AI/agentd-agent-chat-replacement diff --check -- docs/plans/2026-07-09-agentd-native-runtime-roadmap.md
```

Expected: both commands exit 0 with no whitespace errors.

- [x] **Step 4: Review document-only scope**

Run `git status --short` in both repositories and confirm that this task changed only the plan and the two roadmap documents. Do not run Cargo checks because no source or configuration files change.
