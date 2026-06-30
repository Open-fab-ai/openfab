# Design — matrix-Agent execution layer (agent-chat side) + the OpenFab driving contract

Companion to [ROADMAP-verification-thickness.md](ROADMAP-verification-thickness.md). That plan
thickens **OpenFab's verify**; this one designs the **matrix-Agent execution layer** the PPT (S8)
calls for — *agent pooling, capability scheduling, a role matrix that mimics a traditional org*.

Boundary: **OpenFab drives** (asks for "a *coding* agent at *medium* capability, do this task");
**agent-chat schedules** (picks/spins an agent from the pool and runs it). The code below lands
in **agent-chat** (a separate repo); OpenFab only gains a richer dispatch contract.

## Where agent-chat is today
- Each agent record already has a `role` field; agents are named `<team>_<role>`
  (`wf_coordinator`, `wf_implementer`, `wf_reviewer`, `wf_final_reviewer`; plus `alpha_*`,
  `beta_*` teams). Launched via `agentchat up-v1 <name> <runtime>` (claude / codex / …).
- **Missing**: a capability tier, the full 6-role set, a pool, and a capability scheduler. Roles
  are ad-hoc (4) and each agent is a fixed hand-launched session.

## The matrix = role × capability (PPT S8)

**6 roles** (columns) — mimic a traditional org:
`architect · coding · testing · review · integration · documentation`
(map current names: coordinator→architect/orchestrator, implementer→coding, reviewer→review,
final_reviewer→review@strong. Add testing / integration / documentation.)

**3 capability tiers** (rows) — map to runtime/model + cost/latency:
- **strong** — top model (Opus / GPT-5 / Codex), for architect, review, hard coding.
- **medium** — mid model (Sonnet), for most coding / testing / integration.
- **lightweight** — small/fast model (Haiku), for documentation, formatting, trivial edits.

Default role→tier (overridable per task): architect=strong, review=strong, coding=medium,
testing=medium, integration=medium, documentation=lightweight.

## agent-chat changes

### 1. Agent manifest gains `capability` (+ canonical `role`)
Extend the agent record / `up-v1 --role <r> --capability <tier>` and the registration API
(it already normalizes `role`; add `capability ∈ {strong,medium,lightweight}`). `capability`
selects the runtime/model at launch.

### 2. Agent pool + capability index
The backend already keeps an agent registry (`data/agents.json`). Add a query
`GET /api/pool?role=coding&capability=medium&state=idle` → matching online agents. The "pool" is
just the registry indexed by `(role, capability, online, busy)`.

### 3. Capability scheduler — `POST /api/dispatch`
`{ role, capability?, task, room? }` →
1. resolve required tier (explicit `capability`, else the role→tier default);
2. pick an **idle** agent matching `(role, tier)` from the pool;
3. if none: **queue**, or **auto-provision** one (`up-v1` with the tier's runtime) up to a
   per-(role,tier) max-concurrency cap;
4. route the task to it, mark it busy; free it on `task_result`.
This is "按能力调度 / Agent 池化" — one entry point, the pool does the rest.

### 4. Tier → runtime/model table
A small config mapping `capability → runtime/model` (strong→opus|codex, medium→sonnet,
lightweight→haiku), so the scheduler launches the right thing. Cross-model review (OpenFab C14)
just means the review role is staffed from *two different model families* at strong tier.

### 5. Human roles (总监 / 产品 / QA owner)
Unchanged in agent-chat — humans are room members who set direction; the *gate* itself is
OpenFab's N-of-M sign-off. No new agent-chat mechanism needed.

## The OpenFab → matrix-Agent driving contract (OpenFab-side, small)
Today OpenFab's Bridge dispatch targets a fixed `BRIDGE_ASSIGNEE`. Add capability-aware dispatch:
- Bridge env / task: `role` + `capability` (e.g. `role=coding capability=medium`) instead of a
  hard-coded assignee. The Bridge calls agent-chat `POST /api/dispatch {role, capability, ...}`
  and the scheduler picks the agent.
- OpenFab still does NOT care *which* concrete agent ran — it signs the returned bytes. The agent
  identity (name + model family + tier) flows back in the `task_result` and is recorded in the
  AI-BOM (model & agent identity — already a provenance field).
- Workspace mode, caller-mode review, harvest — all unchanged; they target whatever agent the
  scheduler returned.

## Phasing
1. **`capability` field + role→tier defaults + tier→runtime table** (data model only; backward
   compatible — existing named agents keep working).
2. **`GET /api/pool` + `POST /api/dispatch`** scheduler (pick idle by role/tier; queue if none).
3. **Auto-provision** missing (role,tier) agents up to a cap; free on result.
4. **Add the 2 missing roles in active use** (testing, documentation) — staff them; integration
   can fold into coding initially.
5. **OpenFab Bridge** capability-aware dispatch (`role`+`capability`), recording agent identity
   into the AI-BOM.

## Why this stays clean
- OpenFab's trust line is untouched: it still drives, verifies, signs, gates exactly the same —
  it just asks for *capabilities* instead of *named agents*. The execution layer gets richer
  (pool + scheduling + 6 roles) entirely inside agent-chat, which is where matrix-Agent lives.
- Every agent that touches a build is still attributed (name · model family · tier) in the
  signed AI-BOM, so richer staffing doesn't weaken accountability — it sharpens it.
