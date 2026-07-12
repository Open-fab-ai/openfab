spec: task
name: "FSF-0 A: Palpo deployment foundation"
tags: [phase1, fsf0, palpo]
estimate: 2d
---

## Intent

Make the Palpo Matrix homeserver deployment deterministic for local and
small-team use so every other FSF-0 workstream has a stable substrate. Today the
deployment is started by hand; this task produces a repeatable compose profile,
appservice registration, account bootstrap, and health/reset commands in the
demo scaffolding area (`robrix2/roadmap/agentchat-demo`).

## Decisions

- Deployment profile: one compose profile `palpo-local` for the local case and
  the same file parameterized by env for the team-server case.
- Appservice registration: template file `appservice-agentchat.yaml` rendered by
  the deterministic configuration tool; token values come from env, never
  committed, and the sender account is separate from password-login bots.
- Account bootstrap: token-gated `bootstrap-accounts` creates the admin and bot
  accounts, performs one bounded local database promotion for the initial
  server admin, and is idempotent across repeated runs. Open registration is
  forbidden for both local and team-server profiles.
- Health/reset: `demo-doctor` checks homeserver reachability, appservice
  registration match, admin role, and bot account existence; `demo-reset`
  returns the deployment to a clean state.

## Boundaries

### Allowed Changes
- roadmap/agentchat-demo/**

### Forbidden
- Do not commit real tokens, passwords, or signing keys.
- Do not enable unrestricted Matrix registration.
- Do not modify Robrix application source in this task.

## Out of Scope

- Palpo HA topology, SSO/SCIM, federation policy (FSF-7).
- agent-chat service supervision (workstream B).

## Completion Criteria

Scenario: fresh start reaches a healthy homeserver
  Test:
    Package: agentchat-demo
    Filter: test_palpo_fresh_start_healthy
  Given a machine with no prior Palpo state
  When the `palpo-local` compose profile is started and `demo-doctor` runs
  Then `demo-doctor` exits 0 and reports homeserver, appservice, and accounts as present

Scenario: bootstrap-accounts is idempotent
  Test:
    Package: agentchat-demo
    Filter: test_bootstrap_idempotent
  Given `bootstrap-accounts` has already run once
  When `bootstrap-accounts` runs a second time
  Then it exits 0 and the account count is unchanged

Scenario: appservice registration mismatch is diagnosed
  Test:
    Package: agentchat-demo
    Filter: test_doctor_reports_appservice_mismatch
  Given the rendered `appservice-agentchat.yaml` token differs from the homeserver registration
  When `demo-doctor` runs
  Then it exits non-zero and names the appservice registration mismatch as the cause

Scenario: bridge login with a wrong token is rejected and surfaced
  Test:
    Package: agentchat-demo
    Filter: test_wrong_as_token_rejected
  Given a bridge configured with an invalid appservice token
  When the bridge attempts to connect
  Then the homeserver rejects it and `demo-doctor` reports the failing credential

Scenario: demo-reset returns to a clean state
  Test:
    Package: agentchat-demo
    Filter: test_reset_restores_clean_state
  Given a deployment with rooms and bot accounts created
  When `demo-reset` runs and the profile is started again
  Then `demo-doctor` exits 0 and no prior room state remains
