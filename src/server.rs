//! Web server — the visual end-to-end demo (`openfab serve`).
//!
//! A small blocking HTTP server (tiny_http) exposing the spec-cycle as a JSON API and
//! serving an embedded single-page UI. It calls the same `ops` layer the CLI uses, so
//! there is one orchestration code path. The whole UI ships *inside the binary*
//! (`include_str!`), matching OpenFab's single-static-binary / sovereign posture.
//!
//! Long runs (LLM dispatch) execute on a background thread that streams timeline events
//! to disk; the browser polls `…/events`. A global lock serializes the git-touching
//! operations (run/feedback/signoff) so concurrent requests can't corrupt a repo.

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::adapters::registry;
use crate::core::provenance::Attestation;
use crate::core::reputation;
use crate::core::spec::Spec;
use crate::core::trust::Policy;
use crate::ops;
use crate::runstate;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
/// Phase 2 collaborative console (self-contained page: board, docs, stages, agents, identity).
const CONSOLE_HTML: &str = include_str!("../web/console.html");

struct State {
    repo: PathBuf,
    /// Where the multi-project registry + per-project repos live (Phase 2 D).
    projects_dir: PathBuf,
    policy: Policy,
    /// Serializes git-touching operations across requests.
    lock: Mutex<()>,
    /// Background "Run the app" web-server processes, by run id → (pid, port).
    launched: Mutex<HashMap<String, (u32, u16)>>,
}

impl State {
    /// The repo a request targets: the `?project=<name>` workspace, or the default repo.
    fn repo_for(&self, query: &str) -> Result<PathBuf> {
        let name = query_param(query, "project");
        let registry = runstate::load_projects(&self.projects_dir).unwrap_or_default();
        runstate::resolve_project_repo(&registry, name.as_deref(), &self.repo)
    }
}

pub fn serve(repo: PathBuf, port: u16, policy: Policy) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("starting server: {e}"))?;
    let server = Arc::new(server);
    let projects_dir = std::env::var("OPENFAB_PROJECTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            repo.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| repo.clone())
        });
    let state = Arc::new(State {
        repo,
        projects_dir,
        policy,
        lock: Mutex::new(()),
        launched: Mutex::new(HashMap::new()),
    });

    println!("\n  OpenFab web UI →  http://{addr}\n");
    println!("  workspace: {}", state.repo.display());
    println!("  (Ctrl-C to stop)\n");

    let workers = 6;
    let mut handles = vec![];
    for _ in 0..workers {
        let server = server.clone();
        let state = state.clone();
        handles.push(thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle(req, &state);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

fn handle(mut req: Request, state: &Arc<State>) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let query = url.split('?').nth(1).unwrap_or("").to_string();
    let method = req.method().clone();

    let result: Result<Response<std::io::Cursor<Vec<u8>>>> =
        route(&method, &path, &query, &mut req, state);
    let resp = match result {
        Ok(r) => r,
        Err(e) => json_resp(500, &json!({ "error": e.to_string() })),
    };
    let _ = req.respond(resp);
}

fn route(
    method: &Method,
    path: &str,
    query: &str,
    req: &mut Request,
    state: &Arc<State>,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    // CSRF: block cross-origin state-changing requests (localhost binding is not a boundary
    // against the operator's own browser).
    if !matches!(method, Method::Get | Method::Head) && !csrf_ok(req) {
        return Ok(json_resp(
            403,
            &json!({"error":"cross-origin request blocked"}),
        ));
    }
    // The repo a request targets — the `?project=<name>` workspace, or the default repo.
    let repo = state.repo_for(query)?;
    match (method, segs.as_slice()) {
        // --- static UI ---
        (Method::Get, [""]) | (Method::Get, ["index.html"]) => Ok(html(INDEX_HTML)),
        (Method::Get, ["app.js"]) => Ok(asset(APP_JS, "application/javascript")),
        (Method::Get, ["style.css"]) => Ok(asset(STYLE_CSS, "text/css")),
        (Method::Get, ["console"]) | (Method::Get, ["console.html"]) => Ok(html(CONSOLE_HTML)),

        // --- UI config (where the agent-chat monitor lives, etc.) ---
        (Method::Get, ["api", "config"]) => {
            let monitor = std::env::var("OPENFAB_AGENTCHAT_MONITOR")
                .unwrap_or_else(|_| "http://127.0.0.1:8084".to_string());
            let bridged = std::env::var("OPENFAB_AGENTCHAT_URL").ok();
            Ok(json_resp(
                200,
                &json!({ "agentchat_monitor": monitor, "agentchat_bridge": bridged }),
            ))
        }

        // --- catalog ---
        (Method::Get, ["api", "bases"]) => Ok(json_resp(200, &json!(registry::list_bases()))),
        (Method::Get, ["api", "forges"]) => Ok(json_resp(200, &json!(registry::list_forges()))),

        // --- multi-project registry (Phase 2 D) ---
        (Method::Get, ["api", "projects"]) => {
            // Always include the implicit "default" project (the server's --repo workspace).
            let mut list = vec![json!({"name":"default","repo": state.repo.to_string_lossy()})];
            for p in runstate::load_projects(&state.projects_dir)? {
                list.push(json!({"name": p.name, "repo": p.repo}));
            }
            Ok(json_resp(200, &json!(list)))
        }
        (Method::Post, ["api", "projects"]) => {
            let body = body_json(req)?;
            let name = body["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return Ok(json_resp(400, &json!({"error":"name required"})));
            }
            // Each project gets its own repo dir under the projects dir (or an explicit path).
            // With `worktree: true` and an existing git repo, OpenFab creates an isolated git
            // worktree so self-hosting never touches the user's live checkout.
            let want_worktree = body["worktree"].as_bool().unwrap_or(false);
            let src = body["repo"].as_str().filter(|s| !s.is_empty());
            let repo_path = match (src, want_worktree) {
                (Some(r), true) => {
                    runstate::create_worktree(&state.projects_dir, &name, Path::new(r))?
                }
                (Some(r), false) => PathBuf::from(r),
                (None, _) => state.projects_dir.join(&name),
            };
            let proj = runstate::add_project(&state.projects_dir, &name, &repo_path)?;
            Ok(json_resp(200, &json!(proj)))
        }

        // --- Robrix room ↔ project binding + agent doc ingest (Phase 2.1 #3) ---
        (Method::Get, ["api", "rooms"]) => Ok(json_resp(
            200,
            &json!(runstate::load_room_bindings(&state.projects_dir)?),
        )),
        (Method::Post, ["api", "rooms"]) => {
            let body = body_json(req)?;
            let room = body["room"].as_str().unwrap_or("").trim().to_string();
            let project = body["project"].as_str().unwrap_or("").trim().to_string();
            if room.is_empty() || project.is_empty() {
                return Ok(json_resp(
                    400,
                    &json!({"error":"room and project required"}),
                ));
            }
            runstate::bind_room(&state.projects_dir, &room, &project)?;
            Ok(json_resp(200, &json!({"room": room, "project": project})))
        }
        // Ingest a coordinator's finalized docs into a project (by `project` or bound `room`).
        (Method::Post, ["api", "ingest"]) => {
            let body = body_json(req)?;
            // `id` becomes a filename — reject path traversal (it must be a single component).
            let id = match safe_id(body["id"].as_str().unwrap_or("")) {
                Some(id) => id,
                None => return Ok(json_resp(400, &json!({"error":"invalid id"}))),
            };
            // resolve the target project: explicit `project`, else the bound `room`.
            let project = match body["project"].as_str().filter(|s| !s.is_empty()) {
                Some(p) => Some(p.to_string()),
                None => body["room"].as_str().and_then(|room| {
                    let b = runstate::load_room_bindings(&state.projects_dir).unwrap_or_default();
                    runstate::resolve_room_project(&b, room)
                }),
            };
            let reg = runstate::load_projects(&state.projects_dir).unwrap_or_default();
            let target_repo =
                runstate::resolve_project_repo(&reg, project.as_deref(), &state.repo)?;
            // OpenFab ingest writes into the project repo's specs/ (visible on the dashboard).
            let spec_dir = target_repo.join("specs");
            std::fs::create_dir_all(&spec_dir)?;
            let mut wrote = vec![];
            if let Some(req_md) = body["requirements_md"].as_str().filter(|s| !s.is_empty()) {
                let p = spec_dir.join(format!("{id}.requirements.md"));
                std::fs::write(&p, req_md)?;
                wrote.push(format!("{id}.requirements.md"));
            }
            if let Some(spec_md) = body["spec_md"].as_str().filter(|s| !s.is_empty()) {
                let p = spec_dir.join(format!("{id}.spec.md"));
                std::fs::write(&p, spec_md)?;
                wrote.push(format!("{id}.spec.md"));
            }
            Ok(json_resp(
                200,
                &json!({"id": id, "project": project, "wrote": wrote}),
            ))
        }

        // Import a build produced elsewhere (Robrix/agent-chat team) and run it through the gate.
        // POST {id, files:{path:content}, model?, builder?, gate?, project?|room?} → {run_id}.
        // Every build path converges here on OpenFab's verify → sign → conformance → N-of-M gate.
        (Method::Post, ["api", "import-build"]) => {
            let body = body_json(req)?;
            let id = match safe_id(body["id"].as_str().unwrap_or("")) {
                Some(id) => id,
                None => return Ok(json_resp(400, &json!({"error":"invalid id"}))),
            };
            // resolve target project: explicit `project`, else the bound `room` (like ingest).
            let project = match body["project"].as_str().filter(|s| !s.is_empty()) {
                Some(p) => Some(p.to_string()),
                None => body["room"].as_str().and_then(|room| {
                    let b = runstate::load_room_bindings(&state.projects_dir).unwrap_or_default();
                    runstate::resolve_room_project(&b, room)
                }),
            };
            let reg = runstate::load_projects(&state.projects_dir).unwrap_or_default();
            let target_repo =
                runstate::resolve_project_repo(&reg, project.as_deref(), &state.repo)?;
            let spec_file = target_repo.join("specs").join(format!("{id}.spec.md"));
            if !spec_file.exists() {
                return Ok(json_resp(
                    404,
                    &json!({"error": format!("no ingested spec '{id}' in this project — submit the spec first")}),
                ));
            }
            let files: std::collections::BTreeMap<String, String> = body["files"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            if files.is_empty() {
                return Ok(json_resp(400, &json!({"error":"no files to import"})));
            }
            let builder = body["builder"].as_str().unwrap_or("agent-chat").to_string();
            let model = body["model"].as_str().unwrap_or("unknown").to_string();
            let gate = body["gate"].as_str().unwrap_or("team").to_string();
            let run_id = ops::reserve_intent_run_id(&format!("import {id}"));
            seed_status(&target_repo, &run_id, "importing build…", "importing");
            let st = state.clone();
            let (rid, sf) = (run_id.clone(), spec_file.clone());
            thread::spawn(move || {
                let _guard = st.lock.lock().unwrap();
                if let Err(e) = ops::import_build(
                    &target_repo,
                    rid.clone(),
                    Some(sf.as_path()),
                    &builder,
                    &model,
                    files,
                    &gate,
                    &st.policy,
                ) {
                    fail_run(&target_repo, &rid, "importing", &e);
                }
            });
            Ok(json_resp(200, &json!({ "run_id": run_id })))
        }

        // Incoming docs: spec/requirements files in the project (e.g. ingested from Robrix).
        (Method::Get, ["api", "incoming"]) => {
            let spec_dir = repo.join("specs");
            let mut docs = vec![];
            if let Ok(entries) = std::fs::read_dir(&spec_dir) {
                for e in entries.flatten() {
                    let fname = e.file_name().to_string_lossy().to_string();
                    if let Some(id) = fname.strip_suffix(".spec.md") {
                        let has_req = spec_dir.join(format!("{id}.requirements.md")).exists();
                        docs.push(json!({"id": id, "spec": fname, "has_requirements": has_req}));
                    }
                }
            }
            Ok(json_resp(200, &json!(docs)))
        }
        // View one incoming doc's content (spec contract + requirements) before building.
        (Method::Get, ["api", "incoming", id]) => {
            let id = match safe_id(id) {
                Some(id) => id,
                None => return Ok(json_resp(400, &json!({"error":"invalid id"}))),
            };
            let spec_dir = repo.join("specs");
            let spec_md = std::fs::read_to_string(spec_dir.join(format!("{id}.spec.md"))).ok();
            let requirements_md =
                std::fs::read_to_string(spec_dir.join(format!("{id}.requirements.md"))).ok();
            if spec_md.is_none() && requirements_md.is_none() {
                return Ok(json_resp(404, &json!({"error":"no such incoming doc"})));
            }
            Ok(json_resp(
                200,
                &json!({"id": id, "spec_md": spec_md, "requirements_md": requirements_md}),
            ))
        }

        (Method::Get, ["api", "maintainers"]) => {
            Ok(json_resp(200, &json!(runstate::load_maintainers(&repo)?)))
        }
        (Method::Post, ["api", "maintainers"]) => {
            let body = body_json(req)?;
            let name = body["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return Ok(json_resp(400, &json!({"error":"name required"})));
            }
            let (did, new) = runstate::add_maintainer(&repo, &name)?;
            Ok(json_resp(
                200,
                &json!({"name": name, "did": did, "new": new}),
            ))
        }

        // --- identity mapping: Matrix mxid ↔ maintainer (Phase 2 B1) ---
        (Method::Post, ["api", "identity"]) => {
            let body = body_json(req)?;
            let mxid = body["mxid"].as_str().unwrap_or("").trim().to_string();
            let maintainer = body["maintainer"].as_str().unwrap_or("").trim().to_string();
            if mxid.is_empty() || maintainer.is_empty() {
                return Ok(json_resp(
                    400,
                    &json!({"error":"mxid and maintainer required"}),
                ));
            }
            runstate::map_identity(&repo, &mxid, &maintainer)?;
            Ok(json_resp(
                200,
                &json!({"mxid": mxid, "maintainer": maintainer}),
            ))
        }

        // --- upload a requirements / decision doc or a .spec.md (Phase 2.1 #2) ---
        (Method::Post, ["api", "upload"]) => {
            let body = body_json(req)?;
            let name = body["name"].as_str().unwrap_or("upload").to_string();
            let content = body["content"].as_str().unwrap_or("").to_string();
            if content.trim().is_empty() {
                return Ok(json_resp(400, &json!({"error":"empty document"})));
            }
            let spec_dir = std::env::var("OPENFAB_SPEC_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| repo.join("specs"));
            let (id, kind, dest) = ops::save_upload(&spec_dir, &name, &content)?;
            Ok(json_resp(
                200,
                &json!({ "id": id, "kind": kind, "path": dest.to_string_lossy() }),
            ))
        }

        // --- spec authoring (the LLM derives the spec + acceptance from NL) ---
        (Method::Post, ["api", "author"]) => {
            let body = body_json(req)?;
            let intent = body["intent"].as_str().unwrap_or("").to_string();
            let (spec, model, provider) = ops::author_spec(&intent)?;
            Ok(json_resp(
                200,
                &json!({ "spec": spec, "model": model, "provider": provider }),
            ))
        }

        // --- runs ---
        (Method::Post, ["api", "run"]) => start_run(req, state, &repo),
        (Method::Get, ["api", "runs"]) => Ok(json_resp(200, &json!(runstate::list_runs(&repo)?))),
        // Cross-project run history: every project's runs, each tagged with its project, newest
        // first. Lets the dashboard show a complete history regardless of the selected project.
        (Method::Get, ["api", "history"]) => {
            let mut all: Vec<Value> = vec![];
            let mut push_runs = |project: &str, repo: &Path| {
                if let Ok(runs) = runstate::list_runs(repo) {
                    for r in runs {
                        if let Ok(mut v) = serde_json::to_value(&r) {
                            v["project"] = json!(project);
                            all.push(v);
                        }
                    }
                }
            };
            push_runs("default", &state.repo);
            for p in runstate::load_projects(&state.projects_dir).unwrap_or_default() {
                let r = PathBuf::from(&p.repo);
                push_runs(&p.name, &r);
            }
            all.sort_by(|a, b| {
                b.get("created")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(a.get("created").and_then(|v| v.as_str()).unwrap_or(""))
            });
            Ok(json_resp(200, &json!(all)))
        }
        (Method::Get, ["api", "runs", id]) => run_view(id, &repo),
        (Method::Get, ["api", "runs", id, "events"]) => {
            let since = parse_since(query);
            Ok(json_resp(
                200,
                &json!(runstate::read_events(&repo, id, since)),
            ))
        }
        (Method::Get, ["api", "runs", id, "verify"]) => {
            Ok(json_resp(200, &json!(ops::verify(&repo, id)?)))
        }
        (Method::Get, ["api", "runs", id, "artifacts"]) => {
            Ok(json_resp(200, &json!(ops::artifacts(&repo, id)?)))
        }
        (Method::Get, ["api", "runs", id, "audit"]) => {
            Ok(json_resp(200, &json!(ops::audit(&repo, id)?)))
        }
        (Method::Get, ["api", "runs", id, "docs"]) => {
            Ok(json_resp(200, &json!(ops::docs(&repo, id)?)))
        }
        (Method::Get, ["api", "runs", id, "stages"]) => {
            Ok(json_resp(200, &json!(ops::stages(&repo, id)?)))
        }
        (Method::Get, ["api", "board"]) => Ok(json_resp(200, &json!(ops::board(&repo)?))),
        (Method::Get, ["api", "graph"]) => {
            let spec_dir = std::env::var("OPENFAB_SPEC_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| repo.join("specs"));
            Ok(json_resp(
                200,
                &json!({ "dot": ops::spec_graph(&spec_dir)? }),
            ))
        }
        // C2/C3: agent status + tmux peek, proxied (same-origin) to the agent-chat Bridge.
        (Method::Get, ["api", "agents"]) => Ok(json_resp(200, &bridge_get("/agents")?)),
        (Method::Get, ["api", "agents", name, "peek"]) => {
            let q = if query.is_empty() {
                String::new()
            } else {
                format!("?{query}")
            };
            Ok(json_resp(
                200,
                &bridge_get(&format!("/agents/{name}/peek{q}"))?,
            ))
        }
        (Method::Post, ["api", "runs", id, "signoff"]) => {
            let _g = state.lock.lock().unwrap();
            let body = body_json(req)?;
            // Accept either an explicit maintainer name (`as`) or a Matrix user id (`mxid`,
            // Phase 2 Robrix relay). mxid resolves to its mapped maintainer or is rejected.
            let outcome = if let Some(mxid) = body["mxid"].as_str().filter(|s| !s.is_empty()) {
                ops::signoff_by_mxid(&repo, id, mxid, &state.policy)?
            } else {
                let as_name = body["as"].as_str().unwrap_or("").to_string();
                ops::signoff(&repo, id, &as_name, &state.policy)?
            };
            // Close the loop back to Robrix: when the gate opens in the dashboard, notify the
            // bound room (best-effort; needs Matrix/Bridge connected via OPENFAB_AGENTCHAT_URL).
            notify_room_on_signoff(state, query, id, &outcome);
            Ok(json_resp(200, &json!(outcome)))
        }
        (Method::Post, ["api", "runs", id, "reproduce"]) => {
            let _g = state.lock.lock().unwrap();
            Ok(json_resp(
                200,
                &json!(ops::reproduce(&repo, id, &state.policy)?),
            ))
        }
        (Method::Post, ["api", "runs", id, "exec"]) => {
            let _g = state.lock.lock().unwrap();
            let body = body_json(req)?;
            let cmd = body["cmd"].as_str().unwrap_or("").to_string();
            Ok(json_resp(
                200,
                &json!(ops::exec_in_run(&repo, id, &cmd, &state.policy)?),
            ))
        }
        (Method::Post, ["api", "runs", id, "reject"]) => {
            let _g = state.lock.lock().unwrap();
            Ok(json_resp(200, &json!(ops::reject(&repo, id)?)))
        }
        (Method::Post, ["api", "runs", id, "launch"]) => launch_app(id, state, &repo),
        (Method::Post, ["api", "runs", id, "stop"]) => {
            stop_app(id, state);
            Ok(json_resp(200, &json!({ "stopped": true })))
        }
        (Method::Post, ["api", "runs", id, "feedback"]) => feedback(id, req, state, &repo),

        // --- reputation ---
        (Method::Get, ["api", "reputation"]) => reputation_view(&repo),

        _ => Ok(json_resp(
            404,
            &json!({ "error": format!("no route for {path}") }),
        )),
    }
}

/// Start a run. Two shapes:
///   { intent, base, forge, gate }  → the LLM authors the spec, then builds (the UI path)
///   { spec,   base, forge, gate }  → run a pre-authored/saved spec (CLI parity)
fn start_run(
    req: &mut Request,
    state: &Arc<State>,
    repo: &Path,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = body_json(req)?;
    let base = body["base"].as_str().unwrap_or("claude").to_string();
    let forge = body["forge"].as_str().unwrap_or("github").to_string();
    let forge_name = body["forge_name"].as_str().map(String::from);
    let gate = body["gate"].as_str().unwrap_or("solo").to_string();

    // Natural-language path: author + build in the background (in this project's repo).
    if let Some(intent) = body["intent"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let run_id = ops::reserve_intent_run_id(&intent);
        seed_status(repo, &run_id, "authoring spec…", "authoring");
        // An uploaded spec contract (Phase 2.1 #2): build it directly instead of re-drafting.
        // `spec_id` selects a `.spec.md` to build — sanitize it (no path traversal).
        let spec_file = body["spec_id"].as_str().and_then(safe_id).map(|id| {
            let spec_dir = std::env::var("OPENFAB_SPEC_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| repo.join("specs"));
            spec_dir.join(format!("{id}.spec.md"))
        });
        let st = state.clone();
        let repo = repo.to_path_buf();
        let (rid, b, f, fname, g) = (run_id.clone(), base, forge, forge_name, gate);
        thread::spawn(move || {
            let _guard = st.lock.lock().unwrap();
            if let Err(e) = ops::build_with_spec_file(
                &repo,
                &intent,
                rid.clone(),
                &b,
                &f,
                fname,
                &g,
                &st.policy,
                spec_file.as_deref(),
            ) {
                fail_run(&repo, &rid, "authoring", &e);
            }
        });
        return Ok(json_resp(200, &json!({ "run_id": run_id })));
    }

    // Saved-spec path.
    let spec = spec_from_body(&body["spec"])?;
    let run_id = ops::reserve_run_id(&spec);
    seed_status(repo, &run_id, &spec.spec_ref(), "queued");
    spawn_run(
        state.clone(),
        repo.to_path_buf(),
        ops::RunRequest {
            spec,
            base,
            forge_kind: forge,
            forge_name,
            parent_run: None,
            run_id: Some(run_id.clone()),
            gate_mode: gate,
            authored_by: None,
            prebuilt: None,
        },
    );
    Ok(json_resp(200, &json!({ "run_id": run_id })))
}

/// Refine: re-author the spec from the human's feedback and rebuild (v→v+1).
fn feedback(
    id: &str,
    req: &mut Request,
    state: &Arc<State>,
    repo: &Path,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = body_json(req)?;
    let note = body["note"].as_str().unwrap_or("").to_string();
    let base = body["base"].as_str().unwrap_or("claude").to_string();
    let run_id = ops::reserve_refine_run_id(repo, id)?;

    seed_status(repo, &run_id, "re-authoring spec…", "authoring");
    let st = state.clone();
    let repo = repo.to_path_buf();
    let (rid, prior, n, b) = (run_id.clone(), id.to_string(), note, base);
    thread::spawn(move || {
        let _guard = st.lock.lock().unwrap();
        if let Err(e) = ops::refine(&repo, &prior, &n, rid.clone(), &b, &st.policy) {
            fail_run(&repo, &rid, "re-authoring", &e);
        }
    });
    Ok(json_resp(200, &json!({ "run_id": run_id })))
}

fn spawn_run(state: Arc<State>, repo: PathBuf, req: ops::RunRequest) {
    thread::spawn(move || {
        let _g = state.lock.lock().unwrap();
        let run_id = req.run_id.clone().unwrap_or_default();
        let spec_ref = req.spec.spec_ref();
        if let Err(e) = ops::start_run(&repo, req, &state.policy) {
            fail_run(&repo, &run_id, &spec_ref, &e);
        }
    });
}

/// "Run the app": if the product is a web server, launch it on a free port (passing
/// `PORT`) so the user can open it in a browser; otherwise tell the UI it's a CLI (the UI
/// then runs a command). The server is a detached background process, not the sandbox's
/// timed exec, so it can keep serving.
fn launch_app(
    id: &str,
    state: &Arc<State>,
    repo: &Path,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let _g = state.lock.lock().unwrap();
    let rec = runstate::load_run(repo, id)?;
    // Export this run's committed source into an isolated dir, so launching it doesn't
    // disturb the shared working tree and multiple runs' apps can run side-by-side
    // (each serves its OWN version, not whichever branch was checked out last).
    let dest = repo.join(".openfab").join("launch").join(id);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest)?;
    let exported = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "git -C '{}' archive '{}' | tar -x -C '{}'",
            repo.display(),
            rec.branch,
            dest.display()
        ))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !exported {
        anyhow::bail!("could not export the run's source for launch");
    }

    let port = free_port()?;
    let Some((cmd, workdir, file)) = plan_launch(&dest, port) else {
        return Ok(json_resp(200, &json!({ "kind": "cli" })));
    };
    state
        .policy
        .check_command(&cmd)
        .context("launch command refused by policy")?;

    stop_all_apps(state); // only one app runs at a time — kills stale instances (e.g. an old version still serving the old title on an old port)
    let child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .current_dir(&workdir)
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("launching {}", cmd.join(" ")))?;
    let pid = child.id();
    drop(child); // detach — dropping a Child does NOT kill it

    if wait_port(port, Duration::from_secs(5)) {
        state
            .launched
            .lock()
            .unwrap()
            .insert(id.to_string(), (pid, port));
        Ok(json_resp(
            200,
            &json!({ "kind": "web", "url": format!("http://127.0.0.1:{port}"), "file": file, "pid": pid }),
        ))
    } else {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
        Ok(json_resp(
            200,
            &json!({
                "kind": "web-failed",
                "file": file,
                "error": format!("{file} didn't start serving on the port within 5s")
            }),
        ))
    }
}

fn stop_app(id: &str, state: &Arc<State>) {
    if let Some((pid, _)) = state.launched.lock().unwrap().remove(id) {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
}

/// Stop every launched app — so a fresh launch never leaves an old version serving on a
/// stale port (the cause of "I refined but the running app still shows the old title").
fn stop_all_apps(state: &Arc<State>) {
    let mut m = state.launched.lock().unwrap();
    for (_, (pid, _)) in m.drain() {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
}

/// Decide how to run the product in a browser: an actual web server (reads `PORT`), or a
/// static site (serve the dir). A candidate file is only treated as a server if its
/// CONTENTS look like one — so a client-side `app.js`/`index.js` (localStorage, DOM) is
/// served statically, not mistakenly run with `node`. Returns (cmd, workdir, label).
fn plan_launch(repo: &Path, port: u16) -> Option<(Vec<String>, PathBuf, String)> {
    let candidates: [(&str, &str); 12] = [
        ("app/server.py", "python3"),
        ("app/app.py", "python3"),
        ("app/main.py", "python3"),
        ("server.py", "python3"),
        ("app.py", "python3"),
        ("main.py", "python3"),
        ("app/server.js", "node"),
        ("server.js", "node"),
        ("app/app.js", "node"),
        ("app.js", "node"),
        ("app/index.js", "node"),
        ("index.js", "node"),
    ];
    for (f, runner) in candidates {
        let p = repo.join(f);
        if p.exists() && looks_like_server(&p) {
            return Some((vec![runner.into(), f.into()], repo.to_path_buf(), f.into()));
        }
    }
    // Static site: serve the directory that holds index.html (client-side SPAs).
    for dir in ["app", "."] {
        if repo.join(dir).join("index.html").exists() {
            let label = if dir == "." {
                "index.html".into()
            } else {
                format!("{dir}/index.html")
            };
            return Some((
                vec![
                    "python3".into(),
                    "-m".into(),
                    "http.server".into(),
                    port.to_string(),
                    "--bind".into(),
                    "127.0.0.1".into(),
                ],
                repo.join(dir),
                label,
            ));
        }
    }
    None
}

/// True if a source file actually starts an HTTP server (vs. client-side browser JS).
fn looks_like_server(path: &Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    let s = src.to_lowercase();
    let markers = [
        "http.server",
        "httpserver",
        "basehttprequesthandler",
        "socketserver",
        "wsgiref",
        "flask",
        "app.run(",
        "createserver",
        "require('http')",
        "require(\"http\")",
        "from 'http'",
        "from \"http\"",
        "express(",
        ".listen(",
        "deno.serve",
        "bun.serve",
        "socket.bind",
        "asyncio.start_server",
    ];
    markers.iter().any(|m| s.contains(m))
}

fn free_port() -> Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0").context("finding a free port")?;
    Ok(l.local_addr()?.port())
}

fn wait_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(150));
    }
    false
}

/// Record a failed run (event + status) for the UI.
fn fail_run(repo: &Path, run_id: &str, spec_ref: &str, e: &anyhow::Error) {
    runstate::append_event(
        repo,
        run_id,
        &runstate::Event {
            seq: 9999,
            ts: crate::core::timeutil::iso_now(),
            icon: "❌".into(),
            msg: format!("run failed: {e}"),
        },
    );
    runstate::write_status(
        repo,
        &runstate::StatusFile {
            run_id: run_id.to_string(),
            spec_ref: spec_ref.to_string(),
            status: "failed".into(),
            step: "error".into(),
            updated: crate::core::timeutil::iso_now(),
            error: Some(e.to_string()),
        },
    );
}

fn seed_status(repo: &Path, run_id: &str, spec_ref: &str, status: &str) {
    runstate::write_status(
        repo,
        &runstate::StatusFile {
            run_id: run_id.to_string(),
            spec_ref: spec_ref.to_string(),
            status: status.to_string(),
            step: "queued".into(),
            updated: crate::core::timeutil::iso_now(),
            error: None,
        },
    );
}

/// A merged run view: the full record once persisted, else the live status file.
fn run_view(id: &str, repo: &Path) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    if let Ok(rec) = runstate::load_run(repo, id) {
        return Ok(json_resp(200, &json!(rec)));
    }
    match runstate::read_status(repo, id) {
        Some(st) => Ok(json_resp(200, &json!(st))),
        None => Ok(json_resp(404, &json!({ "error": "no such run" }))),
    }
}

fn reputation_view(repo: &Path) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let mut atts = vec![];
    for rec in runstate::list_runs(repo)? {
        if let Ok(text) = std::fs::read_to_string(rec.attestation_path(repo)) {
            if let Ok(att) = Attestation::from_json(&text) {
                atts.push(att);
            }
        }
    }
    let table = reputation::compute(&atts);
    Ok(json_resp(
        200,
        &json!({ "count": atts.len(), "agents": table.values().collect::<Vec<_>>() }),
    ))
}

// --- helpers ---

fn spec_from_body(v: &Value) -> Result<Spec> {
    let yaml = serde_yaml::to_string(v)?;
    Spec::from_yaml(&yaml)
}

/// Reject a request-supplied id/filename stem that could escape its directory. Returns the
/// trimmed id if it is a safe single path component (no separators, no `..`), else None.
fn safe_id(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() || t.contains('/') || t.contains('\\') || t.contains("..") {
        return None;
    }
    Some(t.to_string())
}

/// CSRF guard: reject a state-changing request whose browser `Origin` is cross-site. Requests
/// with no `Origin` (curl, the CLI) pass; the same-origin SPA sends a localhost origin. This
/// stops a malicious page the operator visits from driving the localhost API.
fn csrf_ok(req: &Request) -> bool {
    let origin = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    match origin {
        None => true, // non-browser client (curl / CLI) — no CSRF surface
        Some(o) => o.starts_with("http://127.0.0.1") || o.starts_with("http://localhost"),
    }
}

/// Extract a query-string parameter value (e.g. `project=alpha`), URL-decoding `%xx`/`+`.
/// Notify the Robrix room bound to this run's project when a dashboard sign-off opens the gate.
/// Best-effort and non-fatal: silently does nothing if no Bridge/room is configured or the
/// post fails — the sign-off itself already succeeded.
fn notify_room_on_signoff(
    state: &Arc<State>,
    query: &str,
    run_id: &str,
    outcome: &ops::SignoffOutcome,
) {
    let bridge = match std::env::var("OPENFAB_AGENTCHAT_URL") {
        Ok(b) if !b.is_empty() => b,
        _ => return,
    };
    // Reverse-resolve the room bound to this run's project (default project → OPENFAB_AGENTCHAT_ROOM).
    let project = query_param(query, "project");
    let bindings = runstate::load_room_bindings(&state.projects_dir).unwrap_or_default();
    let room = match &project {
        Some(p) => bindings
            .iter()
            .find(|b| &b.project == p)
            .map(|b| b.room.clone()),
        None => std::env::var("OPENFAB_AGENTCHAT_ROOM").ok(),
    };
    let Some(room) = room else { return };
    let msg = if outcome.accepted {
        format!(
            "✅ OpenFab gate opened for {run_id} — signed off by {}{}. The signed, attributed build is released.",
            outcome.signer_name,
            if outcome.merged { " and merged" } else { "" }
        )
    } else {
        format!(
            "🖊️ {} signed off {run_id} ({} of N). Still awaiting: {}.",
            outcome.signer_name,
            outcome.satisfied.len(),
            outcome.blocking.join(", ")
        )
    };
    let _ = crate::adapters::bridge_client::post_message(&bridge, &room, &msg);
}

fn query_param(query: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(&prefix))
        .map(|v| v.replace('+', " "))
        .map(|v| {
            // minimal percent-decoding (project names are safe identifiers anyway)
            let mut out = String::new();
            let mut bytes = v.bytes();
            while let Some(b) = bytes.next() {
                if b == b'%' {
                    let h: String = bytes.by_ref().take(2).map(|c| c as char).collect();
                    if let Ok(n) = u8::from_str_radix(&h, 16) {
                        out.push(n as char);
                        continue;
                    }
                }
                out.push(b as char);
            }
            out
        })
        .filter(|s| !s.is_empty())
}

fn parse_since(query: &str) -> u64 {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("since="))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Proxy a GET to the agent-chat Bridge (`OPENFAB_AGENTCHAT_URL`) so the dashboard can read
/// agent status / tmux peeks same-origin (no CORS). Uses curl (no HTTP-client crate).
fn bridge_get(path: &str) -> Result<Value> {
    let base = std::env::var("OPENFAB_AGENTCHAT_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("OPENFAB_AGENTCHAT_URL not set (Bridge not configured)"))?;
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let out = Command::new("curl")
        .args(["-sS", "-m", "8", &url])
        .output()
        .context("invoking curl to reach the Bridge")?;
    if !out.status.success() {
        anyhow::bail!(
            "Bridge request failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    serde_json::from_slice(&out.stdout).context("Bridge reply was not JSON")
}

fn body_json(req: &mut Request) -> Result<Value> {
    let mut s = String::new();
    std::io::Read::read_to_string(req.as_reader(), &mut s)?;
    if s.trim().is_empty() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_str(&s)?)
}

fn json_resp(code: u16, v: &Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(v).unwrap_or_default();
    Response::from_data(body)
        .with_status_code(code)
        .with_header(ctype("application/json"))
}

fn html(s: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    asset(s, "text/html; charset=utf-8")
}

fn asset(s: &str, ct: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    // The UI is embedded in the binary and changes across rebuilds; tell the browser not to
    // serve a stale cached copy (a dev tool — correctness over caching).
    let no_cache = Header::from_bytes(&b"Cache-Control"[..], &b"no-cache, must-revalidate"[..])
        .expect("static header");
    Response::from_data(s.as_bytes().to_vec())
        .with_header(ctype(ct))
        .with_header(no_cache)
}

fn ctype(ct: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap()
}
