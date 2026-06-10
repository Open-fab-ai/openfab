# OpenFab trust policy — ILLUSTRATIVE production form (OPA / Rego, evaluated by
# `regorus` in-process per PRD §5). This file documents the *rule shape*. The literal
# parameter values (n_of_m, allowed bases, sandbox lists) live in exactly one place —
# `policy/trust.json` — which both this policy (via `input.policy`) and the v0.1
# in-process evaluator (`core::trust`) read, so there is no duplication (R3).
#
# A change is accepted ⇔ valid fab signature ∧ fab DID allowlisted ∧ base allowlisted
#                        ∧ machine acceptance passed ∧ N-of-M distinct maintainer sign-off.
package openfab.trust

import future.keywords.in

default accept := false

accept if {
	input.signatures.fab_valid
	input.fab_did in input.allowlist.fab
	input.base in input.policy.allowed_bases
	input.acceptance_passed
	count_distinct_maintainer_signoffs >= input.policy.n_of_m.n
}

count_distinct_maintainer_signoffs := n if {
	some valid := {did |
		some s in input.signatures.signoffs
		s.valid
		s.did in input.allowlist.maintainers
	}
	n := count(valid)
}

# The single most sensitive component (PRD §6): the trust gate itself is ALWAYS
# versioned, never hot-loaded, never self-approved. OpenFab cannot autonomously weaken
# its own gate.
deny_self_modify if {
	input.change_touches_trust_gate
	not input.policy.trust_gate_self_modifiable
}
