# ARC Requirements-to-Agent-Chat Spec Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Use ARC only as a requirements compiler/source format: after Robrix/agent-chat confirms requirements with the human, persist an ARC-compatible `requirements/requirements.yaml`, then generate OpenFab/agent-spec contracts that agent-chat implements and converges.

**Architecture:** ARC is not an OpenFab BasePort and must not run design/implement/convergence agents inside OpenFab. The Robrix room conversation remains the requirements authoring surface; after human confirmation, the coordinator submits an ARC-compatible requirement tree to OpenFab. OpenFab stores that tree as the source artifact, compiles it into `requirements.md`, `.spec.md`, and traceability metadata, then hands the contract to the existing agent-chat workflow. OpenFab remains the optional provenance/verification/certification layer; `gate=none` remains the default for Robrix/agent-chat direct work.

**Tech Stack:** Rust 2021, `serde_yaml`, `serde_json`, existing `core::spec::Spec`, existing `adapters::agent_spec` contract shape, existing blocking `tiny_http` server, existing static web UI assets.

## Global Constraints

- Follow `AGENTS.md`: read `docs/OpenFab_MVP_Design_and_PRD.md` first; Core must stay base-agnostic.
- Run after OpenFab code changes: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.
- ARC integration must not add a new agent runtime, BasePort implementation, Python process runner, code generator, or generated-output importer.
- Do not call `arc-agent`, ARC design agents, ARC implement agents, or ARC convergence loops from OpenFab.
- Do not make OpenFab sign-off mandatory for ARC/Robrix/agent-chat work. Imported/certified work still defaults to `gate=none`.
- `agent-chat` remains the execution/convergence layer: coordinator creates work, implementer writes code, reviewer drives fixes, final reviewer or OpenFab verifies when requested.
- `requirements/requirements.yaml` is written only after the coordinator has converged the requirements with the human and the human has confirmed that version.
- OpenFab must not invent `requirements.yaml` from vague NL by itself; it only persists and validates a confirmed ARC-compatible tree submitted by the coordinator or operator.
- Generated `.spec.md` files must be standalone agent-spec Task Contracts with at least two `Scenario:` blocks and bound `Test:` blocks.
- Preserve existing OpenFab `/`, `/console`, `/api/ingest`, `/api/import-build`, and agent-chat bridge behavior.
- No `node_modules`, ARC `.venv`, generated build output, or secrets committed.

---

## File Structure

- Create `src/adapters/arc_authoring.rs`: validate and persist a confirmed ARC-compatible requirement tree as `requirements/requirements.yaml`.
- Create `src/adapters/arc_requirements.rs`: parse ARC `requirements.yaml`, optional `.arc/processing_queue.json`, and optional `.arc/traceability.snapshot.json`.
- Create `src/adapters/arc_spec.rs`: deterministic ARC requirement tree to `requirements.md`, `.spec.md`, `core::spec::Spec`, and traceability manifest compiler.
- Modify `src/adapters/mod.rs`: expose `arc_authoring`, `arc_requirements`, and `arc_spec`.
- Modify `src/cli.rs`: add `openfab arc-spec` command that writes compiled specs into `repo/specs/`.
- Modify `src/server.rs`: add confirmed-requirements ingest API, read-only ARC project API, and spec-generation API; do not add ARC run/import endpoints.
- Create `web/arc.html`: read-only ARC requirement graph and spec-generation panel.
- Create `web/arc.js`: fetch ARC project data, render requirements, call spec-generation API.
- Modify `web/style.css`: styles for the ARC view using existing OpenFab tokens.
- Modify `web/index.html` and `web/console.html`: link to `/arc`.
- Create `docs/ARC-SPEC-ADAPTER.md`: operator runbook and architecture boundary.
- Modify `docs/robrix2-agentchat-integration.md`: describe ARC as a requirements-to-spec source, not an executor.
- Modify `bridge/skills/openfab-coordinator/SKILL.md`: tell coordinators to first converge requirements, then submit ARC-compatible `requirements.yaml`, then preserve generated contracts instead of re-authoring them from scratch.

---

### Task 0: Add Confirmed ARC Requirements Authoring/Ingest

**Files:**
- Create: `src/adapters/arc_authoring.rs`
- Modify: `src/adapters/mod.rs`
- Modify: `src/server.rs`
- Modify: `bridge/skills/openfab-coordinator/SKILL.md`
- Test: inline tests in `src/adapters/arc_authoring.rs`

**Interfaces:**
- Consumes: confirmed ARC-compatible YAML from the coordinator/operator after human requirements acceptance.
- Produces: `write_confirmed_arc_requirements(repo: &Path, requirements_yaml: &str) -> anyhow::Result<ArcRequirementsWriteResult>`
- Produces API: `POST /api/arc/requirements { project?, room?, confirmed:true, requirements_yaml }`
- Writes: `requirements/requirements.yaml`

- [ ] **Step 1: Write the failing authoring test**

Add this test in `src/adapters/arc_authoring.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_confirmed_arc_requirements_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let yaml = r#"
id: ROOT
name: Demo Login App
type: FOLDER
description: A small app with login behavior.
dependencies: []
children:
  - id: REQ-1
    name: Login
    type: ATOMIC
    description: User can log in with valid credentials and sees an error otherwise.
    dependencies: []
    scenarios:
      - name: Valid login
        steps:
          - keyword: GIVEN
            content: A user exists.
          - keyword: WHEN
            content: The user submits valid credentials.
          - keyword: THEN
            content: The session is created.
      - name: Invalid login
        steps:
          - keyword: WHEN
            content: The user submits invalid credentials.
          - keyword: THEN
            content: An error message is shown.
"#;

        let result = write_confirmed_arc_requirements(tmp.path(), yaml).unwrap();
        assert_eq!(result.path, "requirements/requirements.yaml");
        assert_eq!(result.root_id, "ROOT");
        assert_eq!(result.atomic_count, 1);
        assert_eq!(result.scenario_count, 2);
        assert!(tmp.path().join("requirements/requirements.yaml").exists());
    }

    #[test]
    fn rejects_tree_without_scenarios() {
        let tmp = tempfile::tempdir().unwrap();
        let yaml = r#"
id: ROOT
name: Empty
type: FOLDER
description: Missing scenarios.
dependencies: []
children: []
"#;
        let err = write_confirmed_arc_requirements(tmp.path(), yaml).unwrap_err().to_string();
        assert!(err.contains("at least two scenarios"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test arc_authoring -- --nocapture
```

Expected: compile failure because `src/adapters/arc_authoring.rs` does not exist.

- [ ] **Step 3: Implement authoring types and validation**

Add:

```rust
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArcRequirementsWriteResult {
    pub path: String,
    pub root_id: String,
    pub atomic_count: usize,
    pub scenario_count: usize,
}

#[derive(Default)]
struct Counts {
    atomic: usize,
    scenarios: usize,
}

pub fn write_confirmed_arc_requirements(
    repo: &Path,
    requirements_yaml: &str,
) -> Result<ArcRequirementsWriteResult> {
    let tree: Value = serde_yaml::from_str(requirements_yaml)
        .context("confirmed ARC requirements are not valid YAML")?;
    validate_node(&tree, true)?;
    let mut counts = Counts::default();
    count_node(&tree, &mut counts);
    if counts.scenarios < 2 {
        bail!("confirmed ARC requirements must contain at least two scenarios");
    }

    let root_id = tree
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("ROOT")
        .to_string();
    let dir = repo.join("requirements");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("requirements.yaml");
    std::fs::write(&path, requirements_yaml.trim_start())?;

    Ok(ArcRequirementsWriteResult {
        path: "requirements/requirements.yaml".to_string(),
        root_id,
        atomic_count: counts.atomic,
        scenario_count: counts.scenarios,
    })
}
```

- [ ] **Step 4: Implement recursive validation**

Add:

```rust
fn validate_node(node: &Value, is_root: bool) -> Result<()> {
    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
    if id.is_empty() {
        bail!("ARC requirement node is missing id");
    }
    if is_root && id != "ROOT" {
        bail!("ARC requirement root id must be ROOT");
    }
    if name.is_empty() {
        bail!("ARC requirement node '{id}' is missing name");
    }
    if let Some(deps) = node.get("dependencies") {
        if !deps.is_array() {
            bail!("ARC requirement node '{id}' dependencies must be an array");
        }
    }
    if let Some(scenarios) = node.get("scenarios") {
        let Some(scenarios) = scenarios.as_array() else {
            bail!("ARC requirement node '{id}' scenarios must be an array");
        };
        for scenario in scenarios {
            validate_scenario(id, scenario)?;
        }
    }
    if let Some(children) = node.get("children") {
        let Some(children) = children.as_array() else {
            bail!("ARC requirement node '{id}' children must be an array");
        };
        for child in children {
            validate_node(child, false)?;
        }
    }
    Ok(())
}

fn validate_scenario(node_id: &str, scenario: &Value) -> Result<()> {
    let name = scenario
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        bail!("scenario on ARC node '{node_id}' is missing name");
    }
    let steps = scenario
        .get("steps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("scenario '{name}' on ARC node '{node_id}' needs steps[]"))?;
    for step in steps {
        let keyword = step.get("keyword").and_then(|v| v.as_str()).unwrap_or("").trim();
        let content = step.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
        if keyword.is_empty() || content.is_empty() {
            bail!("scenario '{name}' on ARC node '{node_id}' has an empty step");
        }
    }
    Ok(())
}

fn count_node(node: &Value, counts: &mut Counts) {
    let kind = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if kind == "ATOMIC" {
        counts.atomic += 1;
    }
    if let Some(scenarios) = node.get("scenarios").and_then(|v| v.as_array()) {
        counts.scenarios += scenarios.len();
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            count_node(child, counts);
        }
    }
}
```

- [ ] **Step 5: Expose module**

Modify `src/adapters/mod.rs`:

```rust
pub mod arc_authoring;
```

- [ ] **Step 6: Add confirmed-requirements API**

In `src/server.rs`, add route before `/api/arc/spec`:

```rust
(Method::Post, ["api", "arc", "requirements"]) => {
    let body = body_json(req)?;
    if body["confirmed"].as_bool() != Some(true) {
        return Ok(json_resp(
            409,
            &json!({"error":"requirements must be human-confirmed before writing requirements/requirements.yaml"}),
        ));
    }
    let project_name = match body["project"].as_str().filter(|s| !s.is_empty()) {
        Some(p) => Some(p.to_string()),
        None => body["room"].as_str().and_then(|room| {
            let b = runstate::load_room_bindings(&state.projects_dir).unwrap_or_default();
            runstate::resolve_room_project(&b, room)
        }),
    };
    let reg = runstate::load_projects(&state.projects_dir).unwrap_or_default();
    let target_repo = runstate::resolve_project_repo(&reg, project_name.as_deref(), &state.repo)?;
    let yaml = body["requirements_yaml"].as_str().unwrap_or("").trim();
    if yaml.is_empty() {
        return Ok(json_resp(400, &json!({"error":"requirements_yaml required"})));
    }
    let result = crate::adapters::arc_authoring::write_confirmed_arc_requirements(&target_repo, yaml)?;
    Ok(json_resp(200, &json!({
        "project": project_name,
        "path": result.path,
        "root_id": result.root_id,
        "atomic_count": result.atomic_count,
        "scenario_count": result.scenario_count
    })))
}
```

- [ ] **Step 7: Update coordinator skill with the ordering rule**

In `bridge/skills/openfab-coordinator/SKILL.md`, add:

```markdown
## ARC requirements authoring order

When the human wants ARC-backed requirements, do not generate `.spec.md` first.
Follow this sequence:

1. Discuss requirements in the Robrix room.
2. Ask clarifying questions until scope, acceptance scenarios, dependencies, and out-of-scope items are stable.
3. Ask the human to confirm the requirements version.
4. Only after confirmation, submit an ARC-compatible `requirements_yaml` payload to OpenFab `POST /api/arc/requirements` with `confirmed:true`.
5. Then ask OpenFab to compile that file into `specs/<id>.requirements.md`, `specs/<id>.spec.md`, and `specs/<id>.arc.traceability.json`.

`requirements/requirements.yaml` is the source of truth. If requirements change,
return to the room, confirm the new version, rewrite `requirements.yaml`, and
recompile the spec. Do not hand-edit generated `.spec.md` as the primary source.
```

- [ ] **Step 8: Verify**

Run:

```bash
cargo test arc_authoring -- --nocapture
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Expected: all commands pass.

- [ ] **Step 9: Commit**

```bash
git add src/adapters/arc_authoring.rs src/adapters/mod.rs src/server.rs bridge/skills/openfab-coordinator/SKILL.md
git commit -m "feat: ingest confirmed arc requirements"
```

---

### Task 1: Add ARC Requirements Loader

**Files:**
- Create: `src/adapters/arc_requirements.rs`
- Modify: `src/adapters/mod.rs`
- Test: inline tests in `src/adapters/arc_requirements.rs`

**Interfaces:**
- Consumes: `requirements/requirements.yaml`, `.arc/processing_queue.json`, `.arc/traceability.snapshot.json`
- Produces: `ArcRequirementProject`
- Produces: `load_arc_project(repo: &Path) -> anyhow::Result<ArcRequirementProject>`
- Produces: `flatten_requirement_nodes(root: &ArcRequirementNode) -> Vec<&ArcRequirementNode>`

- [ ] **Step 1: Write the failing parser test**

Add this test in `src/adapters/arc_requirements.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_nested_arc_requirements_from_project_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let req_dir = tmp.path().join("requirements");
        std::fs::create_dir_all(&req_dir).unwrap();
        std::fs::write(
            req_dir.join("requirements.yaml"),
            r#"
id: ROOT
name: Demo App
type: FOLDER
description: Demo root
dependencies: []
children:
  - id: REQ-1
    name: Login
    type: ATOMIC
    description: User can log in.
    dependencies: []
    scenarios:
      - name: Valid login
        steps:
          - keyword: GIVEN
            content: A user exists.
          - keyword: WHEN
            content: The user submits credentials.
          - keyword: THEN
            content: The session is created.
"#,
        )
        .unwrap();

        let project = load_arc_project(tmp.path()).unwrap();
        assert_eq!(project.root.id, "ROOT");
        assert_eq!(project.root.children[0].id, "REQ-1");
        assert_eq!(project.sources.requirements_yaml, true);
        assert_eq!(project.sources.processing_queue, false);

        let nodes = flatten_requirement_nodes(&project.root);
        assert_eq!(nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["ROOT", "REQ-1"]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test arc_requirements -- --nocapture
```

Expected: compile failure because `src/adapters/arc_requirements.rs` does not exist.

- [ ] **Step 3: Implement ARC data structures**

Add:

```rust
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArcRequirementNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub scenarios: Vec<ArcScenario>,
    #[serde(default)]
    pub children: Vec<ArcRequirementNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArcScenario {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<ArcScenarioStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArcScenarioStep {
    pub keyword: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ArcSources {
    pub requirements_yaml: bool,
    pub processing_queue: bool,
    pub traceability_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArcRequirementProject {
    pub root: ArcRequirementNode,
    pub statuses: BTreeMap<String, String>,
    pub queue: Option<Value>,
    pub traceability: Option<Value>,
    pub sources: ArcSources,
}
```

- [ ] **Step 4: Implement file loading**

Add:

```rust
pub fn load_arc_project(repo: &Path) -> Result<ArcRequirementProject> {
    let req_path = repo.join("requirements").join("requirements.yaml");
    let text = std::fs::read_to_string(&req_path)
        .with_context(|| format!("reading ARC requirements {}", req_path.display()))?;
    let root: ArcRequirementNode =
        serde_yaml::from_str(&text).context("parsing ARC requirements.yaml")?;

    let queue_path = repo.join(".arc").join("processing_queue.json");
    let queue = read_optional_json(&queue_path)?;

    let trace_path = repo.join(".arc").join("traceability.snapshot.json");
    let traceability = read_optional_json(&trace_path)?;

    let mut statuses = BTreeMap::new();
    for node in flatten_requirement_nodes(&root) {
        statuses.insert(node.id.clone(), "pending".to_string());
    }
    apply_queue_statuses(&mut statuses, queue.as_ref());

    Ok(ArcRequirementProject {
        root,
        statuses,
        queue,
        traceability,
        sources: ArcSources {
            requirements_yaml: true,
            processing_queue: queue_path.exists(),
            traceability_snapshot: trace_path.exists(),
        },
    })
}

fn read_optional_json(path: &Path) -> Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let json = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(json))
}
```

- [ ] **Step 5: Implement flattening and status mapping**

Add:

```rust
pub fn flatten_requirement_nodes(root: &ArcRequirementNode) -> Vec<&ArcRequirementNode> {
    fn walk<'a>(node: &'a ArcRequirementNode, out: &mut Vec<&'a ArcRequirementNode>) {
        out.push(node);
        for child in &node.children {
            walk(child, out);
        }
    }

    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn apply_queue_statuses(statuses: &mut BTreeMap<String, String>, queue: Option<&Value>) {
    let Some(states) = queue
        .and_then(|q| q.get("node_states"))
        .and_then(|v| v.as_object())
    else {
        return;
    };

    for (id, state) in states {
        let mapped = match state.as_str().unwrap_or_default() {
            "DESIGNED" => "designed",
            "PASSED" | "CONVERGED" | "CONVERGED_WITH_FAILED_CHILDREN" => "completed",
            "FAILED" => "failed",
            "RUNNING" => "implementing",
            _ => "pending",
        };
        statuses.insert(id.clone(), mapped.to_string());
    }
}
```

- [ ] **Step 6: Expose module**

Modify `src/adapters/mod.rs`:

```rust
pub mod arc_requirements;
```

- [ ] **Step 7: Verify**

Run:

```bash
cargo test arc_requirements -- --nocapture
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Expected: all commands pass.

- [ ] **Step 8: Commit**

```bash
git add src/adapters/arc_requirements.rs src/adapters/mod.rs
git commit -m "feat: load arc requirements"
```

---

### Task 2: Compile ARC Requirements to OpenFab and agent-spec Contracts

**Files:**
- Create: `src/adapters/arc_spec.rs`
- Modify: `src/adapters/mod.rs`
- Test: inline tests in `src/adapters/arc_spec.rs`

**Interfaces:**
- Consumes: `ArcRequirementProject`
- Produces: `ArcSpecOptions`
- Produces: `ArcSpecBundle`
- Produces: `compile_arc_to_spec(project: &ArcRequirementProject, options: &ArcSpecOptions) -> anyhow::Result<ArcSpecBundle>`

- [ ] **Step 1: Write the failing compiler test**

Add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::arc_requirements::{
        ArcRequirementNode, ArcRequirementProject, ArcScenario, ArcScenarioStep, ArcSources,
    };
    use std::collections::BTreeMap;

    fn sample_project() -> ArcRequirementProject {
        ArcRequirementProject {
            root: ArcRequirementNode {
                id: "ROOT".into(),
                name: "Demo App".into(),
                kind: "FOLDER".into(),
                description: "Build a small login app.".into(),
                dependencies: vec![],
                scenarios: vec![],
                children: vec![ArcRequirementNode {
                    id: "REQ-1".into(),
                    name: "Login".into(),
                    kind: "ATOMIC".into(),
                    description: "User can log in.".into(),
                    dependencies: vec![],
                    scenarios: vec![
                        ArcScenario {
                            name: "Valid login".into(),
                            steps: vec![
                                ArcScenarioStep { keyword: "GIVEN".into(), content: "A user exists.".into() },
                                ArcScenarioStep { keyword: "WHEN".into(), content: "The user submits credentials.".into() },
                                ArcScenarioStep { keyword: "THEN".into(), content: "The session is created.".into() },
                            ],
                        },
                        ArcScenario {
                            name: "Invalid login".into(),
                            steps: vec![
                                ArcScenarioStep { keyword: "WHEN".into(), content: "The user submits bad credentials.".into() },
                                ArcScenarioStep { keyword: "THEN".into(), content: "An error is shown.".into() },
                            ],
                        },
                    ],
                    children: vec![],
                }],
            },
            statuses: BTreeMap::new(),
            queue: None,
            traceability: None,
            sources: ArcSources { requirements_yaml: true, processing_queue: false, traceability_snapshot: false },
        }
    }

    #[test]
    fn compiles_arc_requirements_to_agent_spec_markdown_and_openfab_spec() {
        let bundle = compile_arc_to_spec(
            &sample_project(),
            &ArcSpecOptions {
                id: "demo-login".into(),
                target_dir: "app".into(),
                language: Some("typescript".into()),
                package: None,
                allowed_changes: vec!["app/**".into(), "tests/**".into()],
            },
        )
        .unwrap();

        assert!(bundle.requirements_md.contains("REQ-1"));
        assert!(bundle.spec_md.contains("spec: task"));
        assert!(bundle.spec_md.contains("Scenario: REQ-1 - Valid login"));
        assert!(bundle.spec_md.contains("Filter: test_req_1_valid_login"));
        // Filter-only is the default: `Package:` makes agent-spec run `cargo test -p <pkg>`,
        // which fails when the package name does not exist (hard-won integration lesson).
        assert!(!bundle.spec_md.contains("Package:"));
        assert_eq!(bundle.openfab_spec.id, "demo-login");
        assert_eq!(bundle.openfab_spec.acceptance.len(), 2);
        assert_eq!(bundle.traceability["requirements"][0]["id"], "REQ-1");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test arc_spec -- --nocapture
```

Expected: compile failure because `src/adapters/arc_spec.rs` does not exist.

- [ ] **Step 3: Implement compiler types**

Add:

```rust
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::adapters::arc_requirements::{flatten_requirement_nodes, ArcRequirementNode, ArcRequirementProject};
use crate::core::spec::{Acceptance, Spec};

#[derive(Debug, Clone)]
pub struct ArcSpecOptions {
    pub id: String,
    pub target_dir: String,
    pub language: Option<String>,
    /// Only set when the target workspace really has this package name.
    /// `None` (default) emits Filter-only Test selectors — emitting `Package:`
    /// makes agent-spec run `cargo test -p <pkg>`, which fails when the package
    /// does not exist (documented integration lesson).
    pub package: Option<String>,
    pub allowed_changes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ArcSpecBundle {
    pub requirements_md: String,
    pub spec_md: String,
    pub traceability: Value,
    pub openfab_spec: Spec,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArcSpecWriteResult {
    pub wrote: Vec<String>,
    pub lint: ArcSpecLintStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", content = "message")]
pub enum ArcSpecLintStatus {
    Passed,
    Skipped(String),
}

#[derive(Debug, Serialize)]
struct RequirementTrace {
    id: String,
    name: String,
    scenario_filters: Vec<String>,
}
```

- [ ] **Step 4: Implement `compile_arc_to_spec`**

Add:

```rust
pub fn compile_arc_to_spec(
    project: &ArcRequirementProject,
    options: &ArcSpecOptions,
) -> Result<ArcSpecBundle> {
    if options.id.trim().is_empty() {
        bail!("ARC spec id is required");
    }

    let requirements_md = render_requirements_md(project);
    let scenarios = collect_scenarios(&project.root);
    if scenarios.len() < 2 {
        bail!("ARC requirements must provide at least two scenarios for agent-spec");
    }

    let spec_md = render_agent_spec_md(project, options, &scenarios);
    // Mirror adapters::agent_spec's check-string convention: Filter-only when no
    // package, `pkg::filter` only when one was explicitly provided.
    let acceptance = scenarios
        .iter()
        .map(|s| Acceptance {
            id: s.acceptance_id.clone(),
            check: match &options.package {
                Some(pkg) => format!("agent-spec test: {pkg}::{}", s.filter),
                None => format!("agent-spec test: {}", s.filter),
            },
            must_pass: true,
        })
        .collect();

    let openfab_spec = Spec {
        id: options.id.clone(),
        version: 1,
        intent: render_intent(project),
        context: vec!["ARC requirements/requirements.yaml".into(), format!("specs/{}.spec.md", options.id)],
        acceptance,
        assumptions: vec![
            "ARC is the requirements compiler only; agent-chat remains the implementer/reviewer loop.".into(),
            format!("target_dir: {}", options.target_dir),
        ],
        open_questions: vec![],
        human_signoff_required: false,
        target_dir: options.target_dir.clone(),
        language: options.language.clone(),
    };
    openfab_spec.validate()?;

    let traceability = render_traceability(project, &scenarios);
    Ok(ArcSpecBundle { requirements_md, spec_md, traceability, openfab_spec })
}
```

- [ ] **Step 5: Implement scenario collection**

Add:

```rust
#[derive(Debug, Clone)]
struct CompiledScenario {
    requirement_id: String,
    requirement_name: String,
    scenario_name: String,
    acceptance_id: String,
    filter: String,
    steps: Vec<(String, String)>,
}

fn collect_scenarios(root: &ArcRequirementNode) -> Vec<CompiledScenario> {
    let mut out = Vec::new();
    for node in flatten_requirement_nodes(root) {
        for scenario in &node.scenarios {
            let filter = test_filter(&node.id, &scenario.name);
            out.push(CompiledScenario {
                requirement_id: node.id.clone(),
                requirement_name: node.name.clone(),
                scenario_name: scenario.name.clone(),
                acceptance_id: format!("{} - {}", node.id, scenario.name),
                filter,
                steps: scenario
                    .steps
                    .iter()
                    .map(|s| (s.keyword.clone(), s.content.clone()))
                    .collect(),
            });
        }
    }
    out
}

fn test_filter(requirement_id: &str, scenario_name: &str) -> String {
    let raw = format!("{}_{}", requirement_id, scenario_name);
    let mut out = String::from("test_");
    let mut prev_us = false;
    for c in raw.chars() {
        let next = if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            '_'
        };
        if next == '_' {
            if !prev_us {
                out.push('_');
            }
            prev_us = true;
        } else {
            out.push(next);
            prev_us = false;
        }
    }
    out.trim_end_matches('_').to_string()
}
```

- [ ] **Step 6: Implement markdown rendering**

Add:

```rust
fn render_intent(project: &ArcRequirementProject) -> String {
    format!("{}\n\n{}", project.root.name.trim(), project.root.description.trim()).trim().to_string()
}

fn render_requirements_md(project: &ArcRequirementProject) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", project.root.name));
    out.push_str(project.root.description.trim());
    out.push_str("\n\n## Requirements\n\n");
    for node in flatten_requirement_nodes(&project.root) {
        if node.id == project.root.id {
            continue;
        }
        out.push_str(&format!("### {} - {}\n\n{}\n\n", node.id, node.name, node.description.trim()));
        if !node.dependencies.is_empty() {
            out.push_str(&format!("- Dependencies: {}\n", node.dependencies.join(", ")));
        }
        for scenario in &node.scenarios {
            out.push_str(&format!("- Scenario: {}\n", scenario.name));
            for step in &scenario.steps {
                out.push_str(&format!("  - {} {}\n", step.keyword, step.content));
            }
        }
        out.push('\n');
    }
    out
}

fn render_agent_spec_md(
    project: &ArcRequirementProject,
    options: &ArcSpecOptions,
    scenarios: &[CompiledScenario],
) -> String {
    let mut out = String::new();
    out.push_str("spec: task\n");
    out.push_str(&format!("name: \"{}\"\n", options.id));
    out.push_str("tags: [\"arc\", \"agent-chat\"]\n");
    out.push_str("---\n\n");
    out.push_str("## Intent\n\n");
    out.push_str(&render_intent(project));
    out.push_str("\n\n## Decisions\n\n");
    out.push_str("- ARC is used only to compile requirements into this contract.\n");
    out.push_str("- agent-chat is the execution, review, and convergence loop.\n");
    out.push_str("- OpenFab sign-off is optional unless the caller explicitly requests a gate.\n");
    if let Some(language) = &options.language {
        out.push_str(&format!("- Primary implementation language: {}\n", language));
    }
    out.push_str("\n## Boundaries\n\n### Allowed Changes\n");
    for glob in &options.allowed_changes {
        out.push_str(&format!("- {}\n", glob));
    }
    out.push_str("\n### Forbidden\n\n");
    out.push_str("- Do not run ARC design, implement, or convergence agents for this task.\n");
    out.push_str("- Do not replace the agent-chat issue/review workflow.\n");
    out.push_str("\n## Completion Criteria\n\n");
    for scenario in scenarios {
        out.push_str(&format!("Scenario: {}\n", scenario.acceptance_id));
        out.push_str("  Test:\n");
        if let Some(package) = &options.package {
            out.push_str(&format!("    Package: {}\n", package));
        }
        out.push_str(&format!("    Filter: {}\n", scenario.filter));
        for (keyword, content) in &scenario.steps {
            let keyword = normalize_bdd_keyword(keyword);
            out.push_str(&format!("  {} {}\n", keyword, content.trim()));
        }
        out.push('\n');
    }
    out.push_str("## Out of Scope\n\n");
    out.push_str("- ARC-generated application code.\n");
    out.push_str("- ARC agent runtime integration.\n");
    out
}

fn normalize_bdd_keyword(keyword: &str) -> &'static str {
    match keyword.trim().to_ascii_uppercase().as_str() {
        "GIVEN" => "Given",
        "WHEN" => "When",
        "THEN" => "Then",
        "AND" => "And",
        _ => "And",
    }
}
```

- [ ] **Step 7: Implement traceability rendering**

Add:

```rust
fn render_traceability(project: &ArcRequirementProject, scenarios: &[CompiledScenario]) -> Value {
    let requirements: Vec<RequirementTrace> = flatten_requirement_nodes(&project.root)
        .into_iter()
        .filter(|n| n.id != project.root.id)
        .map(|n| RequirementTrace {
            id: n.id.clone(),
            name: n.name.clone(),
            scenario_filters: scenarios
                .iter()
                .filter(|s| s.requirement_id == n.id)
                .map(|s| s.filter.clone())
                .collect(),
        })
        .collect();

    json!({
        "source": "arc.requirements",
        "execution_owner": "agent-chat",
        "certification_owner": "openfab",
        "requirements": requirements,
    })
}
```

- [ ] **Step 8: Expose module**

Modify `src/adapters/mod.rs`:

```rust
pub mod arc_spec;
```

- [ ] **Step 9: Verify**

Run:

```bash
cargo test arc_spec -- --nocapture
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Expected: all commands pass.

- [ ] **Step 10: Commit**

```bash
git add src/adapters/arc_spec.rs src/adapters/mod.rs
git commit -m "feat: compile arc requirements to specs"
```

---

### Task 3: Add CLI and API for Spec Generation Only

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/server.rs`
- Test: inline tests for write helper in `src/server.rs` or `src/adapters/arc_spec.rs`

**Interfaces:**
- Produces CLI: `openfab arc-spec --repo <path> --id <id> --target-dir app`
- Optional CLI: `--package <real-package-name>` only for workspaces where that package exists.
- Produces API: `POST /api/arc/spec`
- Writes:
  - `specs/<id>.requirements.md`
  - `specs/<id>.spec.md`
  - `specs/<id>.arc.traceability.json`

- [ ] **Step 1: Add persistence helper test**

Add a helper test near the implementation location:

```rust
#[test]
fn writes_arc_spec_bundle_under_specs_dir() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("OPENFAB_ARC_SPEC_LINT", "0");
    let bundle = ArcSpecBundle {
        requirements_md: "# Demo\n".into(),
        spec_md: "spec: task\nname: \"demo\"\ntags: []\n---\n\n## Intent\n\nDemo\n".into(),
        traceability: serde_json::json!({"requirements":[]}),
        openfab_spec: crate::core::spec::Spec {
            id: "demo".into(),
            version: 1,
            intent: "Demo".into(),
            context: vec![],
            acceptance: vec![crate::core::spec::Acceptance {
                id: "s1".into(),
                check: "agent-spec test: test_s1".into(),
                must_pass: true,
            }],
            assumptions: vec![],
            open_questions: vec![],
            human_signoff_required: false,
            target_dir: "app".into(),
            language: None,
        },
    };

    let result = write_arc_spec_bundle(tmp.path(), "demo", &bundle).unwrap();
    assert_eq!(result.wrote, vec![
        "demo.requirements.md",
        "demo.spec.md",
        "demo.arc.traceability.json",
    ]);
    assert!(matches!(result.lint, ArcSpecLintStatus::Skipped(_)));
    assert!(tmp.path().join("specs/demo.spec.md").exists());
    std::env::remove_var("OPENFAB_ARC_SPEC_LINT");
}
```

- [ ] **Step 2: Implement persistence helper with lint gate**

Add to `src/adapters/arc_spec.rs`:

```rust
pub fn write_arc_spec_bundle(repo: &Path, id: &str, bundle: &ArcSpecBundle) -> Result<ArcSpecWriteResult> {
    let spec_dir = repo.join("specs");
    std::fs::create_dir_all(&spec_dir)?;

    let lint = lint_arc_spec_md(&spec_dir, id, &bundle.spec_md)?;

    let req_name = format!("{id}.requirements.md");
    let spec_name = format!("{id}.spec.md");
    let trace_name = format!("{id}.arc.traceability.json");

    std::fs::write(spec_dir.join(&req_name), &bundle.requirements_md)?;
    std::fs::write(spec_dir.join(&spec_name), &bundle.spec_md)?;
    std::fs::write(
        spec_dir.join(&trace_name),
        serde_json::to_string_pretty(&bundle.traceability)?,
    )?;

    Ok(ArcSpecWriteResult {
        wrote: vec![req_name, spec_name, trace_name],
        lint,
    })
}

fn lint_arc_spec_md(spec_dir: &Path, id: &str, spec_md: &str) -> Result<ArcSpecLintStatus> {
    if std::env::var("OPENFAB_ARC_SPEC_LINT").ok().as_deref() == Some("0") {
        return Ok(ArcSpecLintStatus::Skipped(
            "disabled by OPENFAB_ARC_SPEC_LINT=0".to_string(),
        ));
    }

    let bin = std::env::var("OPENFAB_AGENT_SPEC_BIN").unwrap_or_else(|_| "agent-spec".to_string());
    let draft = spec_dir.join(format!(".{id}.arc-lint.spec.md"));
    std::fs::write(&draft, spec_md)
        .with_context(|| format!("writing lint draft {}", draft.display()))?;
    let draft_str = draft.to_string_lossy().to_string();
    let out = match Command::new(&bin)
        .args(["lint", draft_str.as_str(), "--format", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let _ = std::fs::remove_file(&draft);
            return Ok(ArcSpecLintStatus::Skipped(format!(
                "{bin} not found; skipped agent-spec lint"
            )));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&draft);
            return Err(e).with_context(|| format!("running {bin} lint"));
        }
    };
    let _ = std::fs::remove_file(&draft);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lint_json: Value = serde_json::from_str(&stdout).with_context(|| {
        format!(
            "`{bin} lint` did not emit JSON.\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })?;
    let min_score = std::env::var("OPENFAB_SPEC_MIN_SCORE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.7);
    crate::adapters::agent_spec::lint_gate(&lint_json, min_score)?;
    Ok(ArcSpecLintStatus::Passed)
}
```

- [ ] **Step 3: Add ignored live lint smoke test**

Create `tests/arc_spec_lint_smoke.rs`:

```rust
//! Live smoke: ARC-generated `.spec.md` should pass the real agent-spec lint gate.
//! Ignored by default because it needs the `agent-spec` CLI installed.
//!
//! Run explicitly:
//!   cargo test --test arc_spec_lint_smoke -- --ignored --nocapture

use openfab::adapters::agent_spec::lint_gate;

#[test]
#[ignore = "live: needs the agent-spec CLI"]
fn arc_generated_spec_lints_with_real_agent_spec() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("demo.spec.md");
    std::fs::write(
        &spec,
        r#"spec: task
name: "demo"
tags: ["arc"]
---

## Intent

Demo behavior.

## Decisions

- Filter-only tests.

## Boundaries

### Allowed Changes
- app/**

### Forbidden
- No ARC implementation agents.

## Completion Criteria

Scenario: happy path
  Test:
    Filter: test_happy_path
  Given valid input
  When I run the app
  Then it succeeds

Scenario: error path
  Test:
    Filter: test_error_path
  Given invalid input
  When I run the app
  Then it reports an error

## Out of Scope

- ARC-generated application code.
"#,
    )
    .unwrap();
    let bin = std::env::var("OPENFAB_AGENT_SPEC_BIN").unwrap_or_else(|_| "agent-spec".to_string());
    let spec_str = spec.to_string_lossy().to_string();
    let out = std::process::Command::new(&bin)
        .args(["lint", spec_str.as_str(), "--format", "json"])
        .output()
        .expect("agent-spec lint failed to run");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    lint_gate(&json, 0.7).unwrap();
}
```

- [ ] **Step 4: Add CLI command enum variant**

Modify `src/cli.rs`:

```rust
ArcSpec {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    id: String,
    #[arg(long, default_value = "app")]
    target_dir: String,
    /// Optional package name. Omit by default so generated contracts use Filter-only tests.
    #[arg(long)]
    package: Option<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long = "allow", default_values_t = vec!["app/**".to_string(), "tests/**".to_string()])]
    allowed_changes: Vec<String>,
},
```

- [ ] **Step 5: Wire CLI command**

Add match arm:

```rust
Cmd::ArcSpec { repo, id, target_dir, package, language, allowed_changes } => {
    cmd_arc_spec(&repo, &id, &target_dir, package, language, allowed_changes)
}
```

Add function:

```rust
fn cmd_arc_spec(
    repo: &Path,
    id: &str,
    target_dir: &str,
    package: Option<String>,
    language: Option<String>,
    allowed_changes: Vec<String>,
) -> Result<()> {
    let repo = abs(repo)?;
    let project = crate::adapters::arc_requirements::load_arc_project(&repo)?;
    let bundle = crate::adapters::arc_spec::compile_arc_to_spec(
        &project,
        &crate::adapters::arc_spec::ArcSpecOptions {
            id: id.to_string(),
            target_dir: target_dir.to_string(),
            language,
            package,
            allowed_changes,
        },
    )?;
    let result = crate::adapters::arc_spec::write_arc_spec_bundle(&repo, id, &bundle)?;
    println!("compiled ARC requirements into specs/:");
    for file in result.wrote {
        println!("  {file}");
    }
    println!("agent-spec lint: {:?}", result.lint);
    Ok(())
}
```

- [ ] **Step 6: Add read-only ARC project API**

In `src/server.rs`, add route:

```rust
(Method::Get, ["api", "arc", "project"]) => {
    let req_path = repo.join("requirements").join("requirements.yaml");
    if !req_path.exists() {
        return Ok(json_resp(404, &json!({"error":"no ARC requirements found"})));
    }
    let project = crate::adapters::arc_requirements::load_arc_project(&repo)?;
    Ok(json_resp(200, &serde_json::to_value(project)?))
}
```

Missing `requirements/requirements.yaml` returns HTTP 404 so the UI can show a normal empty state instead of an internal-error toast.

- [ ] **Step 7: Add spec-generation API**

In `src/server.rs`, add route:

```rust
(Method::Post, ["api", "arc", "spec"]) => {
    let body = body_json(req)?;
    let id = match safe_id(body["id"].as_str().unwrap_or("")) {
        Some(id) => id,
        None => return Ok(json_resp(400, &json!({"error":"invalid id"}))),
    };
    let project_name = match body["project"].as_str().filter(|s| !s.is_empty()) {
        Some(p) => Some(p.to_string()),
        None => body["room"].as_str().and_then(|room| {
            let b = runstate::load_room_bindings(&state.projects_dir).unwrap_or_default();
            runstate::resolve_room_project(&b, room)
        }),
    };
    let reg = runstate::load_projects(&state.projects_dir).unwrap_or_default();
    let target_repo = runstate::resolve_project_repo(&reg, project_name.as_deref(), &state.repo)?;
    let req_path = target_repo.join("requirements").join("requirements.yaml");
    if !req_path.exists() {
        return Ok(json_resp(404, &json!({"error":"no ARC requirements found"})));
    }
    let project = crate::adapters::arc_requirements::load_arc_project(&target_repo)?;
    let bundle = crate::adapters::arc_spec::compile_arc_to_spec(
        &project,
        &crate::adapters::arc_spec::ArcSpecOptions {
            id: id.clone(),
            target_dir: body["target_dir"].as_str().unwrap_or("app").to_string(),
            language: body["language"].as_str().map(|s| s.to_string()),
            package: body["package"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string),
            allowed_changes: body["allowed_changes"]
                .as_array()
                .map(|xs| xs.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_else(|| vec!["app/**".to_string(), "tests/**".to_string()]),
        },
    )?;
    let result = crate::adapters::arc_spec::write_arc_spec_bundle(&target_repo, &id, &bundle)?;
    Ok(json_resp(200, &json!({
        "id": id,
        "project": project_name,
        "wrote": result.wrote,
        "lint": result.lint,
        "spec": format!("specs/{id}.spec.md"),
        "requirements": format!("specs/{id}.requirements.md")
    })))
}
```

- [ ] **Step 8: Verify CLI and API**

Run:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -- serve --repo demo/.work/web --port 8787 --policy policy/trust.json
```

In a second terminal, submit the confirmed requirements source, then verify CLI and API compilation:

```bash
REQ_YAML=$(cat <<'YAML'
id: ROOT
name: Demo Login App
type: FOLDER
description: A small app with login behavior.
dependencies: []
children:
  - id: REQ-1
    name: Login
    type: ATOMIC
    description: User can log in with valid credentials and sees an error otherwise.
    dependencies: []
    scenarios:
      - name: Valid login
        steps:
          - keyword: GIVEN
            content: A user exists.
          - keyword: WHEN
            content: The user submits valid credentials.
          - keyword: THEN
            content: The session is created.
      - name: Invalid login
        steps:
          - keyword: WHEN
            content: The user submits invalid credentials.
          - keyword: THEN
            content: An error message is shown.
YAML
)
jq -n --arg yaml "$REQ_YAML" '{confirmed:true, requirements_yaml:$yaml}' \
  | curl -s -X POST http://127.0.0.1:8787/api/arc/requirements \
      -H 'Content-Type: application/json' \
      -d @- | jq .
cargo run -- arc-spec --repo demo/.work/web --id demo-arc-cli --target-dir app
curl -s -X POST http://127.0.0.1:8787/api/arc/spec \
  -H 'Content-Type: application/json' \
  -d '{"id":"demo-arc","target_dir":"app"}' | jq .
```

Expected:
- first three commands pass;
- `/api/arc/requirements` writes `requirements/requirements.yaml` only when `confirmed:true`;
- `arc-spec` writes three files into `demo/.work/web/specs/` when ARC requirements exist;
- generated `.spec.md` contains `Filter:` selectors and no `Package:` by default;
- API returns `{"id":"demo-arc","wrote":[...],"lint":...}`;
- no command invokes `arc-agent`.

- [ ] **Step 9: Commit**

```bash
git add src/cli.rs src/server.rs src/adapters/arc_spec.rs tests/arc_spec_lint_smoke.rs
git commit -m "feat: generate specs from arc requirements"
```

---

### Task 4: Add Read-Only ARC View in OpenFab UI

**Files:**
- Create: `web/arc.html`
- Create: `web/arc.js`
- Modify: `src/server.rs`
- Modify: `web/index.html`
- Modify: `web/console.html`
- Modify: `web/style.css`

**Interfaces:**
- Consumes: `GET /api/arc/project`
- Consumes: `POST /api/arc/spec`
- Produces: `/arc`
- Produces: `/arc.js`

- [ ] **Step 1: Add embedded assets**

Modify `src/server.rs`:

```rust
const ARC_HTML: &str = include_str!("../web/arc.html");
const ARC_JS: &str = include_str!("../web/arc.js");
```

Add routes:

```rust
(Method::Get, ["arc"]) | (Method::Get, ["arc.html"]) => Ok(html(ARC_HTML)),
(Method::Get, ["arc.js"]) => Ok(asset(ARC_JS, "application/javascript")),
```

- [ ] **Step 2: Create `web/arc.html`**

Create:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width,initial-scale=1" />
    <title>OpenFab ARC Requirements</title>
    <link rel="stylesheet" href="/style.css" />
  </head>
  <body class="arc-page">
    <header class="topbar">
      <a class="brand" href="/">OpenFab</a>
      <nav>
        <a href="/console">Console</a>
        <a href="/arc" aria-current="page">ARC Requirements</a>
      </nav>
    </header>
    <main class="arc-shell">
      <section class="arc-toolbar">
        <div>
          <h1>ARC Requirements</h1>
          <p id="arc-source">requirements/requirements.yaml</p>
        </div>
        <form id="arc-spec-form">
          <input name="id" placeholder="spec id" required />
          <input name="target_dir" value="app" />
          <input name="package" placeholder="package (optional)" />
          <button type="submit">Generate Spec</button>
        </form>
      </section>
      <section id="arc-status" class="arc-status"></section>
      <section id="arc-tree" class="arc-tree"></section>
    </main>
    <script src="/arc.js" type="module"></script>
  </body>
</html>
```

- [ ] **Step 3: Create `web/arc.js`**

Create:

```javascript
const tree = document.querySelector('#arc-tree');
const status = document.querySelector('#arc-status');
const form = document.querySelector('#arc-spec-form');

async function jsonFetch(url, options) {
  const res = await fetch(url, options);
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(json.error || `${res.status} ${res.statusText}`);
  return json;
}

function renderNode(node, statuses, depth = 0) {
  const state = statuses?.[node.id] || 'pending';
  const scenarios = (node.scenarios || [])
    .map((s) => `<li>${escapeHtml(s.name)}</li>`)
    .join('');
  const children = (node.children || [])
    .map((child) => renderNode(child, statuses, depth + 1))
    .join('');
  return `
    <article class="arc-node" style="--depth:${depth}">
      <div class="arc-node-head">
        <strong>${escapeHtml(node.id)} - ${escapeHtml(node.name)}</strong>
        <span class="badge">${escapeHtml(state)}</span>
      </div>
      <p>${escapeHtml(node.description || '')}</p>
      ${scenarios ? `<ul>${scenarios}</ul>` : ''}
      ${children}
    </article>`;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

async function load() {
  try {
    const project = await jsonFetch('/api/arc/project');
    status.textContent = 'Loaded ARC requirements. Generate Spec writes specs/<id>.spec.md for agent-chat.';
    tree.innerHTML = renderNode(project.root, project.statuses || {});
  } catch (error) {
    status.textContent = `No ARC requirements loaded: ${error.message}`;
    tree.innerHTML = '';
  }
}

form.addEventListener('submit', async (event) => {
  event.preventDefault();
  const body = Object.fromEntries(new FormData(form).entries());
  status.textContent = 'Generating spec...';
  try {
    const result = await jsonFetch('/api/arc/spec', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const lint = result.lint?.status ? ` lint=${result.lint.status}` : '';
    status.textContent = `Wrote ${result.wrote.join(', ')}.${lint} Use the generated .spec.md with agent-chat.`;
  } catch (error) {
    status.textContent = `Spec generation failed: ${error.message}`;
  }
});

load();
```

- [ ] **Step 4: Add CSS**

Modify `web/style.css`:

```css
.arc-page {
  background: var(--bg);
  color: var(--ink);
}

.arc-shell {
  max-width: 1180px;
  margin: 0 auto;
  padding: 24px;
}

.arc-toolbar {
  display: flex;
  gap: 16px;
  justify-content: space-between;
  align-items: flex-end;
  border-bottom: 1px solid var(--line);
  padding-bottom: 16px;
}

.arc-toolbar form {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.arc-status {
  min-height: 24px;
  margin: 16px 0;
  color: var(--muted);
}

.arc-tree {
  display: grid;
  gap: 10px;
}

.arc-node {
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 12px;
  margin-left: calc(var(--depth) * 18px);
  background: var(--panel);
}

.arc-node-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
```

- [ ] **Step 5: Link from existing UI**

Modify `web/index.html` inside `<div class="navlinks">` next to the existing Fabricate/Console links:

```html
<a class="navlink" href="/arc">ARC Requirements</a>
```

Modify `web/console.html` inside `<div class="navlinks">` next to the existing Fabricate/Console links:

```html
<a class="navlink" href="/arc">ARC Requirements</a>
```

Use the existing nav/link style in each file rather than creating a new visual system. Do not modify `web/app.js` for navigation; the dashboard nav is static in `web/index.html`.

- [ ] **Step 6: Verify**

Run:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
node --check web/arc.js
rg -n 'strip_suffix\\("\\.spec\\.md"\\)' src/server.rs
```

Manual:

```bash
cargo run -- serve --repo demo/.work/web --port 8787 --policy policy/trust.json
open http://127.0.0.1:8787/arc
```

Expected:
- `/arc` loads without VS Code APIs;
- missing ARC files show a readable empty/error state;
- existing `/` and `/console` still load;
- clicking `Generate Spec` writes only spec artifacts, not application code.
- Incoming docs still key only off `*.spec.md`; `*.requirements.md` and `*.arc.traceability.json` are not listed as separate build entries.

- [ ] **Step 7: Commit**

```bash
git add src/server.rs web/arc.html web/arc.js web/index.html web/console.html web/style.css
git commit -m "feat: add arc requirements view"
```

---

### Task 5: Preserve agent-chat as the Execution and Convergence Loop

**Files:**
- Modify: `docs/robrix2-agentchat-integration.md`
- Modify: `bridge/skills/openfab-coordinator/SKILL.md`
- Create: `docs/ARC-SPEC-ADAPTER.md`

**Interfaces:**
- Consumes: `requirements/requirements.yaml`
- Consumes: `specs/<id>.requirements.md`
- Consumes: `specs/<id>.spec.md`
- Produces operator workflow docs for Robrix/agent-chat using ARC-generated contracts.

- [ ] **Step 1: Update Robrix integration doc**

Add a section to `docs/robrix2-agentchat-integration.md`:

```markdown
## ARC requirements source

ARC may be used as a requirements compiler before agent-chat execution:

1. Robrix + agent-chat coordinator owns requirements conversation and human confirmation.
2. After confirmation, OpenFab persists the coordinator-submitted ARC-compatible tree as
   `requirements/requirements.yaml`.
3. OpenFab `arc-spec` compiles that graph into `specs/<id>.requirements.md`,
   `specs/<id>.spec.md`, and `specs/<id>.arc.traceability.json`.
4. agent-chat owns implementation and convergence from the `.spec.md` contract.
5. OpenFab import/certification remains opt-in and defaults to `gate=none`.

OpenFab must not run ARC design/implement agents or import ARC-generated application code.
```

- [ ] **Step 2: Update coordinator skill**

In `bridge/skills/openfab-coordinator/SKILL.md`, add:

```markdown
## ARC-generated contracts

For ARC-backed work, follow this order:

1. Discuss requirements in the Robrix room.
2. Ask clarifying questions until scope, dependencies, acceptance scenarios, and out-of-scope items are stable.
3. Ask the human to confirm that requirements version.
4. Submit the confirmed ARC-compatible YAML to OpenFab `/api/arc/requirements` with `confirmed:true`.
5. Compile the persisted `requirements/requirements.yaml` into `specs/<id>.requirements.md`,
   `specs/<id>.spec.md`, and `specs/<id>.arc.traceability.json`.
6. Create/route the agent-chat issue using the generated `.spec.md`.

If the project already has generated `specs/<id>.requirements.md` and
`specs/<id>.spec.md`, treat those files as derived from the accepted
`requirements/requirements.yaml`. Do not re-author the contract from scratch
unless the human asks for a requirements revision. Preserve requirement ids in
the task briefing and send reviewer feedback back to the same issue until the
contract converges.
```

- [ ] **Step 3: Create operator runbook**

Create `docs/ARC-SPEC-ADAPTER.md`:

```markdown
# ARC Spec Adapter

ARC is a requirements format/source for OpenFab, not an execution base.

## Ownership

| Layer | Owner |
| --- | --- |
| Requirements conversation | Robrix room + agent-chat coordinator |
| Confirmed requirement graph | `requirements/requirements.yaml` persisted by OpenFab |
| Contract generation | OpenFab ARC adapter |
| Implementation/review/convergence | agent-chat room workflow |
| Optional provenance/certification | OpenFab |

## Author confirmed requirements

The coordinator writes `requirements/requirements.yaml` only after the human has
confirmed the requirements version. OpenFab persists the confirmed source via:

```bash
REQ_YAML=$(cat requirements/requirements.yaml)
jq -n --arg yaml "$REQ_YAML" '{confirmed:true, requirements_yaml:$yaml}' \
  | curl -s -X POST http://127.0.0.1:8787/api/arc/requirements \
      -H 'Content-Type: application/json' \
      -d @- | jq .
```

## Generate contracts

```bash
cargo run -- arc-spec \
  --repo demo/.work/web \
  --id demo-arc \
  --target-dir app
```

Use `--package <real-package-name>` only when the target repo is a known multi-package
workspace and that package name exists. The default is Filter-only verification.

Expected files:

```text
specs/demo-arc.requirements.md
specs/demo-arc.spec.md
specs/demo-arc.arc.traceability.json
```

## Execute with agent-chat

Use the generated `.spec.md` as the agent-chat task contract. The implementer
writes code, the reviewer checks it, and failed review returns to the same task.

## Certify with OpenFab

When the room has produced code, use the existing `/api/import-build` path with
`gate=none` unless the project explicitly wants a human release gate.

## Non-goals

- Running ARC design agents from OpenFab.
- Running ARC implement agents from OpenFab.
- Importing ARC-generated application code.
- Making OpenFab sign-off mandatory for Robrix/agent-chat rooms.
```

- [ ] **Step 4: Verify docs references**

Run:

```bash
rg -n "ARC|arc-spec|requirements.yaml|gate=none|agent-chat" docs bridge/skills/openfab-coordinator/SKILL.md
rg -n "arc_runner|import-latest|POST /api/arc/compile|ARC Output Import" docs/superpowers/plans/2026-07-07-arc-ui-in-openfab.md docs/ARC-SPEC-ADAPTER.md docs/robrix2-agentchat-integration.md bridge/skills/openfab-coordinator/SKILL.md
```

Expected:
- first command shows the new documented flow;
- second command only matches in explicit non-goal text if it matches at all.

- [ ] **Step 5: Verify full repo**

Run:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
git diff --check
```

Expected: all commands pass.

- [ ] **Step 6: Commit**

```bash
git add docs/ARC-SPEC-ADAPTER.md docs/robrix2-agentchat-integration.md bridge/skills/openfab-coordinator/SKILL.md
git commit -m "docs: document arc spec adapter workflow"
```

---

## End-to-End Smoke Test

After Tasks 0-5, start OpenFab in one terminal:

```bash
cargo run -- serve --repo demo/.work/web --port 8787 --policy policy/trust.json
```

In a second terminal, submit confirmed requirements, compile them, and inspect the generated contract:

```bash
REQ_YAML=$(cat <<'YAML'
id: ROOT
name: Demo Login App
type: FOLDER
description: A small app with login behavior.
dependencies: []
children:
  - id: REQ-1
    name: Login
    type: ATOMIC
    description: User can log in with valid credentials and sees an error otherwise.
    dependencies: []
    scenarios:
      - name: Valid login
        steps:
          - keyword: GIVEN
            content: A user exists.
          - keyword: WHEN
            content: The user submits valid credentials.
          - keyword: THEN
            content: The session is created.
      - name: Invalid login
        steps:
          - keyword: WHEN
            content: The user submits invalid credentials.
          - keyword: THEN
            content: An error message is shown.
YAML
)

jq -n --arg yaml "$REQ_YAML" '{confirmed:true, requirements_yaml:$yaml}' \
  | curl -s -X POST http://127.0.0.1:8787/api/arc/requirements \
      -H 'Content-Type: application/json' \
      -d @- | jq .

curl -s -X POST http://127.0.0.1:8787/api/arc/spec \
  -H 'Content-Type: application/json' \
  -d '{"id":"demo-login","target_dir":"app"}' | jq .

test -f demo/.work/web/requirements/requirements.yaml
test -f demo/.work/web/specs/demo-login.spec.md
test -f demo/.work/web/specs/demo-login.requirements.md
test -f demo/.work/web/specs/demo-login.arc.traceability.json
rg -n "Scenario: REQ-1 - Valid login|Filter: test_req_1_valid_login" demo/.work/web/specs/demo-login.spec.md
```

Expected:
- `requirements/requirements.yaml` is created only through the confirmed requirements ingest path;
- the three spec artifacts exist;
- `.spec.md` has two scenarios with bound test filters;
- `.spec.md` has no `Package:` line by default;
- no application code is generated;
- agent-chat can use `specs/demo-login.spec.md` as its implementation contract.

## Self-Review

**Spec coverage:** This plan now covers the full needed ARC capability in order: Robrix/agent-chat requirements confirmation, confirmed `requirements/requirements.yaml` authoring, requirement loading, deterministic contract generation, UI inspection, and agent-chat handoff. It removes ARC execution/import tasks that would duplicate agent-chat.

**Placeholder scan:** No implementation step uses placeholder wording. Every introduced function has a signature and consuming task.

**Type consistency:** `ArcRequirementsWriteResult`, `write_confirmed_arc_requirements`, `ArcRequirementProject`, `ArcSpecOptions`, `ArcSpecBundle`, `ArcSpecWriteResult`, `ArcSpecLintStatus`, `compile_arc_to_spec`, and `write_arc_spec_bundle` are introduced before API/CLI/UI tasks consume them.

**Boundary check:** ARC is not a BasePort. OpenFab does not run ARC agents. agent-chat remains the implementation/review/convergence system. OpenFab sign-off remains optional and defaults to `gate=none` for direct Robrix/agent-chat work.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-07-arc-ui-in-openfab.md`.

Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints.
