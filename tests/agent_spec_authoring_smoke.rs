//! Live smoke test: the deterministic half of agent-spec authoring — take a `.spec.md`,
//! gate it with real `agent-spec lint`, parse it with real `agent-spec parse`, map it to
//! OpenFab's `Spec`, and persist it. Exercises the subprocess integration + mapping end to
//! end. Ignored by default (needs the `agent-spec` CLI installed).
//!
//! Run explicitly:
//!   cargo test --test agent_spec_authoring_smoke -- --ignored --nocapture

const SPEC_MD: &str = r#"spec: task
name: "int-adder-cli"
tags: []
---

## Intent

A command-line tool that adds two integers passed as arguments and prints their sum.

## Decisions

- Python 3 standard library only

## Boundaries

### Allowed Changes
- app/**

### Forbidden
- No third-party dependencies

## Completion Criteria

Scenario: adds two positive integers
  Test:
    Package: app
    Filter: test_adds_two_positives
  Given the adder tool
  When I run it with 2 and 3
  Then it prints 5

Scenario: rejects a non-integer argument
  Test:
    Package: app
    Filter: test_rejects_non_integer
  Given the adder tool
  When I run it with a non-integer argument
  Then it exits non-zero with an error

## Out of Scope

- Floating point input
"#;

#[test]
#[ignore = "live: needs the agent-spec CLI"]
fn lints_parses_maps_and_persists_a_spec_md() {
    let tmp = tempfile::tempdir().unwrap();
    let authored = openfab::adapters::agent_spec::author_from_md(
        SPEC_MD,
        "add two integers",
        tmp.path(),
        "hand-written".to_string(),
        "test".to_string(),
        None,
    )
    .expect("author_from_md failed");

    // persisted under the canonical id, draft cleaned up
    assert!(
        authored.spec_md_path.exists(),
        "the .spec.md was not persisted"
    );
    assert_eq!(authored.contract.spec.id, "int-adder-cli");
    assert!(
        !tmp.path().join(".openfab-draft.spec.md").exists(),
        "draft should have been renamed away"
    );

    // mapping: both scenarios → acceptance; decisions + boundaries preserved
    let spec = &authored.contract.spec;
    assert_eq!(spec.acceptance.len(), 2, "expected two scenarios");
    assert!(authored
        .contract
        .decisions
        .iter()
        .any(|d| d.contains("Python 3")));
    assert!(authored.contract.allow.iter().any(|a| a == "app/**"));
    assert!(authored
        .contract
        .deny
        .iter()
        .any(|d| d.contains("third-party")));

    eprintln!("--- persisted: {} ---", authored.spec_md_path.display());
    eprintln!(
        "--- acceptance ids: {:?} ---",
        spec.acceptance
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>()
    );
    eprintln!(
        "--- folded assumptions: {:?} ---",
        authored.contract.folded_spec().assumptions
    );
}
