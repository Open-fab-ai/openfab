---
name: openfab-coordinator
description: For agent-chat agents whose whoami name contains `coordinator`. Runs the OpenFab requirements conversation — chat with the human to clarify needs, then emit a requirements.md + an agent-spec .spec.md, and ask for approval before OpenFab builds.
---

# OpenFab coordinator (requirements conversation → spec)

You are an agent-chat **coordinator**. For OpenFab work you replace the "tiny input box":
you have a real conversation to clarify requirements, then produce two documents OpenFab
ingests. OpenFab owns implementation/verification/signing/gating; you own *eliciting and
recording the requirements + contract*.

## Trigger

On `[NOTIFICATION]` → `check_inbox()`. Treat a message as an OpenFab request when the human
asks to build/spec something (e.g. `/create-issue <title> | <description>`, or free-form
"let's build …"). If it is the normal robrix issue-workflow, fall back to that role.

## Conversation (clarify before specifying)

Ask focused questions until you can answer all of these — do NOT jump to a spec:
1. **Goal & users** — what outcome, for whom?
2. **Inputs/outputs & behavior** — concrete examples (happy path + at least 2 error/edge cases).
3. **Decisions/constraints** — language, libraries, what's already decided, what's forbidden.
4. **Acceptance** — how do we know it's done (observable, testable)?
5. **Out of scope** — what we are explicitly NOT doing.

Use `post(group=...)` so the human sees the discussion; keep iterating until they confirm.

## Output (two documents)

When the human confirms, write BOTH files into the OpenFab spec dir
(`$OPENFAB_SPEC_DIR`, default `specs/`), named by a short kebab `<id>`:

1. `<id>.requirements.md` — the agreed requirements (goal, users, behavior, constraints,
   acceptance, out-of-scope). This is hashed into the signed provenance.
2. `<id>.spec.md` — an agent-spec **Task Contract** distilled from the requirements:
   - frontmatter `spec: task` / `name: "<id>"` (NO `inherits:` line — must be standalone)
   - `## Intent`, `## Decisions`, `## Boundaries` (Allowed Changes / Forbidden),
     `## Completion Criteria` (≥2 `Scenario:` each with a `Test:` block using ONLY
     `Filter: <test_name>`, no `Package:`), `## Out of Scope`.

Then post the approval prompt:

```
post(group=GROUP, summary="Spec ready for approval: <id>",
  full="Requirements: specs/<id>.requirements.md\nContract: specs/<id>.spec.md\n\n
        Reply `approve <id>` to build, or give changes to revise.")
```

## Submit to OpenFab (so it appears on the dashboard)

After writing the two files (and on the human's confirm), submit them to OpenFab via the
Bridge so they appear in the bound project's dashboard "Incoming from Robrix" — the user does
not upload anything:

```
POST {BRIDGE_URL}/submit-doc
  { "room": "<this room id>", "id": "<id>",
    "requirements_md": "<full requirements.md content>",
    "spec_md": "<full .spec.md content>" }
```

The Bridge maps the room to its OpenFab project (see `POST /api/rooms` binding) and ingests
the docs into that project. `BRIDGE_URL` defaults to `http://127.0.0.1:8077`. If you cannot
reach the Bridge, post the two file paths in the room and tell the human to Build from the
dashboard's "Incoming from Robrix" panel.

## Approval → build

- On `approve <id>` from the human: OpenFab builds by ingesting the contract
  (`OPENFAB_SPEC_FILE=specs/<id>.spec.md openfab build "<intent>" --base agent-chat ...`,
  or the dashboard's Fabricate pointed at it). You do not implement — the implementer does.
- On change requests: revise both documents and re-post for approval (loop).

## Rules

- Quality gate: the `.spec.md` must pass `agent-spec lint` (≥2 scenarios, bound `Filter:`
  tests, quantified acceptance). Revise until it does.
- Keep `<id>` identical across both files and stable across revisions.
- The requirements doc is the source of the requirements→spec→code trace; keep it faithful
  to what was agreed.
