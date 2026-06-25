spec: task
name: "openfab-matrix-identity-mapping"
tags: []
---

## Intent

Map a Matrix user (mxid) to an OpenFab maintainer so that approving/signing in a Robrix room
becomes an N-of-M sign-off, while ensuring only a mapped (and thus authorized) Matrix user
can sign — a bare `approve` from an unmapped room member must never produce a signature.

## Decisions

- Extend `MaintainerEntry` with an optional `mxid`; the mapping lives in the existing
  maintainer allowlist file (single source of truth for who may sign).
- Resolution is a pure function over the allowlist: exactly one mapped maintainer → ok;
  none → reject; multiple → ambiguous/reject.

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not let an unmapped mxid resolve to a signer.

## Completion Criteria

Scenario: a mapped Matrix user resolves to its maintainer
  Test:
    Filter: test_resolve_signer_maps_known_mxid
  Given a maintainer allowlist where alice is mapped to @alice:palpo
  When resolving the signer for @alice:palpo
  Then it returns the maintainer alice

Scenario: an unmapped Matrix user is rejected (cannot sign)
  Test:
    Filter: test_resolve_signer_rejects_unmapped_mxid
  Given a maintainer allowlist with no mapping for @mallory:palpo
  When resolving the signer for @mallory:palpo
  Then resolution fails and no signer is returned

Scenario: an ambiguous mapping is rejected
  Test:
    Filter: test_resolve_signer_rejects_ambiguous_mxid
  Given two maintainers both mapped to the same mxid
  When resolving the signer for that mxid
  Then resolution fails

## Out of Scope

- The Bridge relay that calls sign-off (task B2) and the dashboard identity UI.
