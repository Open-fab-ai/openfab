# FSF-0A Palpo Real E2E Candidate Evidence

- Evidence captured: `2026-07-12T21:39:13+08:00`
- Result: **CANDIDATE ONLY**
- Human sign-off: not recorded
- Robrix2 base revision: `3d273cbca2da938e73afe42334332bd834f465e4`
- Robrix2 branch: `main` (dirty worktree; implementation is not committed)
- Palpo revision: `b5aaa17645b27db335563cc61e58582ad98b00e0`
- Palpo branch/worktree: `main`, clean
- Docker server: `29.4.0`
- Palpo implementation aggregate SHA-256:
  `1696f24b012cb000f853340f09abfe5d4220a850333c8ffa594decf6526028d2`

The aggregate digest is the SHA-256 of the sorted per-file SHA-256 output for
`roadmap/agentchat-demo/palpo/**`, excluding `.runtime/**`. It identifies the
tested bytes but is not a substitute for an immutable source commit.

## Command

```bash
PALPO_REAL_E2E=1 node --test \
  roadmap/agentchat-demo/palpo/tests/real-e2e.test.mjs
```

No Claude, Codex, agent-chat, OpenFab, or other agent runtime was started. Each
scenario used an isolated Compose project, random credentials, and an isolated
runtime directory. The harness removed its containers, network, volumes, and
runtime files after each scenario.

## Result

```text
PASS test_palpo_fresh_start_healthy
PASS test_bootstrap_idempotent
PASS test_doctor_reports_appservice_mismatch
PASS test_wrong_as_token_rejected
PASS test_reset_restores_clean_state
tests 5; pass 5; fail 0; duration_ms 148406.249542
```

The reset scenario created a Matrix room, stopped the isolated profile, removed
rendered configuration and Palpo/Postgres state, rendered and started a clean
profile, bootstrapped accounts again, passed doctor, and verified that the old
room was not accessible.

## Supporting Gates

- Default hermetic suite: 28 passed, 5 real selectors explicitly skipped.
- `bash -n demo-reset.sh`: passed.
- `docker compose ... config --quiet`: passed without starting services.
- FSF-0A task contract: parse passed; lint quality 100%.
- Residual resources: no `agentchat-palpo-e2e` containers remained.

## Remaining Acceptance Work

- Human operator review and sign-off.
- Commit or signed artifact digest for the exact Robrix2 source bytes.
- FSF-0 workstreams B-E and the cross-system acceptance evidence remain open.
