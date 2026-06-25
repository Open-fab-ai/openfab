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
use crate::spec_cycle::RunMode;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

struct State {
    repo: PathBuf,
    policy: Policy,
    /// Serializes git-touching operations across requests.
    lock: Mutex<()>,
    /// Background "Run the app" web-server processes, by run id → (pid, port).
    launched: Mutex<HashMap<String, (u32, u16)>>,
}

pub fn serve(repo: PathBuf, port: u16, policy: Policy) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("starting server: {e}"))?;
    let server = Arc::new(server);
    let state = Arc::new(State {
        repo,
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
    match (method, segs.as_slice()) {
        // --- static UI ---
        (Method::Get, [""]) | (Method::Get, ["index.html"]) => Ok(html(INDEX_HTML)),
        (Method::Get, ["app.js"]) => Ok(asset(APP_JS, "application/javascript")),
        (Method::Get, ["style.css"]) => Ok(asset(STYLE_CSS, "text/css")),

        // --- catalog ---
        (Method::Get, ["api", "bases"]) => Ok(json_resp(200, &json!(registry::list_bases()))),
        // Bring a base's native runtime up (so the user can run it for real, not bridged).
        (Method::Post, ["api", "base", id, "launch"]) => match registry::launch_base(id) {
            Ok(outcome) => Ok(json_resp(200, &json!(outcome))),
            Err(e) => Ok(json_resp(400, &json!({ "error": e.to_string() }))),
        },
        (Method::Get, ["api", "forges"]) => Ok(json_resp(200, &json!(registry::list_forges()))),
        (Method::Get, ["api", "maintainers"]) => Ok(json_resp(
            200,
            &json!(runstate::load_maintainers(&state.repo)?),
        )),
        (Method::Post, ["api", "maintainers"]) => {
            let body = body_json(req)?;
            let name = body["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return Ok(json_resp(400, &json!({"error":"name required"})));
            }
            let (did, new) = runstate::add_maintainer(&state.repo, &name)?;
            Ok(json_resp(
                200,
                &json!({"name": name, "did": did, "new": new}),
            ))
        }

        // --- spec authoring (the LLM derives the spec + acceptance from NL) ---
        (Method::Post, ["api", "author"]) => {
            let body = body_json(req)?;
            let intent = body["intent"].as_str().unwrap_or("").to_string();
            let author_model = body["author_model"].as_str().filter(|s| !s.is_empty());
            let (spec, model, provider) = ops::author_spec(&intent, author_model)?;
            Ok(json_resp(
                200,
                &json!({ "spec": spec, "model": model, "provider": provider }),
            ))
        }
        // Models available on the configured Ollama endpoint (key stays server-side).
        (Method::Get, ["api", "models"]) => {
            match crate::adapters::llm_backend::list_ollama_models() {
                Ok(models) => Ok(json_resp(200, &json!({ "models": models }))),
                Err(e) => Ok(json_resp(
                    200,
                    &json!({ "models": [], "error": e.to_string() }),
                )),
            }
        }

        // --- runs ---
        (Method::Post, ["api", "run"]) => start_run(req, state),
        (Method::Get, ["api", "runs"]) => {
            Ok(json_resp(200, &json!(runstate::list_runs(&state.repo)?)))
        }
        (Method::Get, ["api", "runs", id]) => run_view(id, state),
        (Method::Get, ["api", "runs", id, "events"]) => {
            let since = parse_since(query);
            Ok(json_resp(
                200,
                &json!(runstate::read_events(&state.repo, id, since)),
            ))
        }
        (Method::Get, ["api", "runs", id, "verify"]) => {
            Ok(json_resp(200, &json!(ops::verify(&state.repo, id)?)))
        }
        (Method::Get, ["api", "runs", id, "artifacts"]) => {
            Ok(json_resp(200, &json!(ops::artifacts(&state.repo, id)?)))
        }
        (Method::Get, ["api", "runs", id, "audit"]) => {
            Ok(json_resp(200, &json!(ops::audit(&state.repo, id)?)))
        }
        (Method::Post, ["api", "runs", id, "signoff"]) => {
            let _g = state.lock.lock().unwrap();
            let body = body_json(req)?;
            let as_name = body["as"].as_str().unwrap_or("").to_string();
            Ok(json_resp(
                200,
                &json!(ops::signoff(&state.repo, id, &as_name, &state.policy)?),
            ))
        }
        (Method::Post, ["api", "runs", id, "reproduce"]) => {
            let _g = state.lock.lock().unwrap();
            Ok(json_resp(
                200,
                &json!(ops::reproduce(&state.repo, id, &state.policy)?),
            ))
        }
        (Method::Post, ["api", "runs", id, "exec"]) => {
            let _g = state.lock.lock().unwrap();
            let body = body_json(req)?;
            let cmd = body["cmd"].as_str().unwrap_or("").to_string();
            Ok(json_resp(
                200,
                &json!(ops::exec_in_run(&state.repo, id, &cmd, &state.policy)?),
            ))
        }
        (Method::Post, ["api", "runs", id, "reject"]) => {
            let _g = state.lock.lock().unwrap();
            Ok(json_resp(200, &json!(ops::reject(&state.repo, id)?)))
        }
        (Method::Post, ["api", "runs", id, "launch"]) => launch_app(id, state),
        (Method::Post, ["api", "runs", id, "stop"]) => {
            stop_app(id, state);
            Ok(json_resp(200, &json!({ "stopped": true })))
        }
        (Method::Post, ["api", "runs", id, "feedback"]) => feedback(id, req, state),
        (Method::Post, ["api", "runs", id, "promote"]) => promote_run(id, state),

        // --- apps (each intent = a new app; refines are versions of it) ---
        (Method::Get, ["api", "apps"]) => Ok(json_resp(200, &json!(ops::apps(&state.repo)?))),
        (Method::Delete, ["api", "apps", id]) => {
            let _g = state.lock.lock().unwrap();
            Ok(json_resp(
                200,
                &json!({ "deleted": ops::delete_app(&state.repo, id)? }),
            ))
        }
        (Method::Post, ["api", "apps", id, "open"]) => open_app(id, state),
        // Open ALL of a run's artifacts (source + provenance + run-state) in the file manager.
        (Method::Post, ["api", "runs", id, "open"]) => {
            let _g = state.lock.lock().unwrap();
            let dir = ops::export_run_bundle(&state.repo, id)?;
            let _ = Command::new("open").arg(&dir).status();
            Ok(json_resp(
                200,
                &json!({ "path": dir.display().to_string() }),
            ))
        }

        // --- reputation ---
        (Method::Get, ["api", "reputation"]) => reputation_view(state),

        _ => Ok(json_resp(
            404,
            &json!({ "error": format!("no route for {path}") }),
        )),
    }
}

/// Start a run. Two shapes:
///   { intent, base, forge, gate }  → the LLM authors the spec, then builds (the UI path)
///   { spec,   base, forge, gate }  → run a pre-authored/saved spec (CLI parity)
fn start_run(req: &mut Request, state: &Arc<State>) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = body_json(req)?;
    let base = body["base"].as_str().unwrap_or("claude").to_string();
    let forge = body["forge"].as_str().unwrap_or("github").to_string();
    let forge_name = body["forge_name"].as_str().map(String::from);
    let gate = body["gate"].as_str().unwrap_or("solo").to_string();
    let mode = if body["mode"].as_str() == Some("draft") {
        RunMode::Draft
    } else {
        RunMode::Release
    };
    let allow_bridged = body["allow_bridged"].as_bool().unwrap_or(false);
    let author_model = body["author_model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    let base_model = body["base_model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);

    // Pre-flight: if the chosen base's native runtime isn't running and the user hasn't
    // opted into the bridged stand-in, refuse up-front with a structured response so the UI
    // can offer to launch it (or run bridged) — never a silent substitution (R14).
    if let Some(resp) = base_unavailable_response(&base, allow_bridged) {
        return Ok(resp);
    }

    // Natural-language path: author + build in the background.
    if let Some(intent) = body["intent"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let run_id = ops::reserve_intent_run_id(&intent);
        seed_status(state, &run_id, "authoring spec…", "authoring");
        let st = state.clone();
        let (rid, b, f, fname, g) = (run_id.clone(), base, forge, forge_name, gate);
        let (am, bm) = (author_model, base_model);
        thread::spawn(move || {
            let _guard = st.lock.lock().unwrap();
            if let Err(e) = ops::build(
                &st.repo,
                &intent,
                rid.clone(),
                &b,
                &f,
                fname,
                &g,
                mode,
                allow_bridged,
                am.as_deref(),
                bm,
                &st.policy,
            ) {
                fail_run(&st, &rid, "authoring", &e);
            }
        });
        return Ok(json_resp(200, &json!({ "run_id": run_id })));
    }

    // Saved-spec path.
    let spec = spec_from_body(&body["spec"])?;
    let run_id = ops::reserve_run_id(&spec);
    seed_status(state, &run_id, &spec.spec_ref(), "queued");
    spawn_run(
        state.clone(),
        ops::RunRequest {
            spec,
            base,
            forge_kind: forge,
            forge_name,
            parent_run: None,
            run_id: Some(run_id.clone()),
            gate_mode: gate,
            authored_by: None,
            mode,
            allow_bridged,
            base_model,
        },
    );
    Ok(json_resp(200, &json!({ "run_id": run_id })))
}

/// If `base` is a framework whose native runtime is unreachable and the user did not opt
/// into bridging, return a 409 with everything the UI needs to offer Launch / bridged.
/// Returns `None` when the run may proceed (base reachable, bridging allowed, or claude).
fn base_unavailable_response(
    base: &str,
    allow_bridged: bool,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    if allow_bridged {
        return None;
    }
    let st = registry::base_status(base);
    if !st.is_framework || st.reachable {
        return None;
    }
    let display = base; // id doubles as a fine label here
    Some(json_resp(
        409,
        &json!({
            "error_kind": "base_unavailable",
            "base": st.id,
            "display": display,
            "launchable": st.launchable,
            "endpoint": st.endpoint,
            "hint": if st.launchable {
                format!("{display} is not running. Launch it, or run with the bridged stand-in.")
            } else {
                format!("{display} is not running and has no bundled launcher. Start its adapter, or run bridged.")
            },
        }),
    ))
}

/// Refine: re-author the spec from the human's feedback and rebuild (v→v+1).
fn feedback(
    id: &str,
    req: &mut Request,
    state: &Arc<State>,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = body_json(req)?;
    let note = body["note"].as_str().unwrap_or("").to_string();
    let base = body["base"].as_str().unwrap_or("claude").to_string();
    let mode = if body["mode"].as_str() == Some("draft") {
        RunMode::Draft
    } else {
        RunMode::Release
    };
    let allow_bridged = body["allow_bridged"].as_bool().unwrap_or(false);
    let author_model = body["author_model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    let base_model = body["base_model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    if let Some(resp) = base_unavailable_response(&base, allow_bridged) {
        return Ok(resp);
    }
    let run_id = ops::reserve_refine_run_id(&state.repo, id)?;

    seed_status(state, &run_id, "re-authoring spec…", "authoring");
    let st = state.clone();
    let (rid, prior, n, b) = (run_id.clone(), id.to_string(), note, base);
    let (am, bm) = (author_model, base_model);
    thread::spawn(move || {
        let _guard = st.lock.lock().unwrap();
        if let Err(e) = ops::refine(
            &st.repo,
            &prior,
            &n,
            rid.clone(),
            &b,
            mode,
            allow_bridged,
            am.as_deref(),
            bm,
            &st.policy,
        ) {
            fail_run(&st, &rid, "re-authoring", &e);
        }
    });
    Ok(json_resp(200, &json!({ "run_id": run_id })))
}

/// Promote a draft to a signed release — the full ceremony, run in the background so the
/// UI streams it (the new release run id is returned for polling).
fn promote_run(id: &str, state: &Arc<State>) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let run_id = ops::reserve_promote_run_id(&state.repo, id)?;
    seed_status(
        state,
        &run_id,
        "promoting draft → signed release…",
        "queued",
    );
    let st = state.clone();
    let (rid, draft) = (run_id.clone(), id.to_string());
    thread::spawn(move || {
        let _g = st.lock.lock().unwrap();
        if let Err(e) = ops::promote(&st.repo, &draft, rid.clone(), &st.policy) {
            fail_run(&st, &rid, "promote", &e);
        }
    });
    Ok(json_resp(200, &json!({ "run_id": run_id })))
}

/// Open an app's source folder in the OS file manager (macOS Finder via `open`).
fn open_app(id: &str, state: &Arc<State>) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let _g = state.lock.lock().unwrap();
    let app = ops::apps(&state.repo)?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| anyhow::anyhow!("no such app"))?;
    let dir = ops::export_app_dir(&state.repo, &app.latest_run)?;
    let _ = Command::new("open").arg(&dir).status();
    Ok(json_resp(
        200,
        &json!({ "path": dir.display().to_string() }),
    ))
}

fn spawn_run(state: Arc<State>, req: ops::RunRequest) {
    thread::spawn(move || {
        let _g = state.lock.lock().unwrap();
        let run_id = req.run_id.clone().unwrap_or_default();
        let spec_ref = req.spec.spec_ref();
        if let Err(e) = ops::start_run(&state.repo, req, &state.policy) {
            fail_run(&state, &run_id, &spec_ref, &e);
        }
    });
}

/// "Run the app": if the product is a web server, launch it on a free port (passing
/// `PORT`) so the user can open it in a browser; otherwise tell the UI it's a CLI (the UI
/// then runs a command). The server is a detached background process, not the sandbox's
/// timed exec, so it can keep serving.
fn launch_app(id: &str, state: &Arc<State>) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let _g = state.lock.lock().unwrap();
    let rec = runstate::load_run(&state.repo, id)?;
    // Export this run's committed source into an isolated dir, so launching it doesn't
    // disturb the shared working tree and multiple runs' apps can run side-by-side
    // (each serves its OWN version, not whichever branch was checked out last).
    let dest = state.repo.join(".openfab").join("launch").join(id);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest)?;
    let exported = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "git -C '{}' archive '{}' | tar -x -C '{}'",
            state.repo.display(),
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
    // Static site FIRST: most generated web apps are client-side SPAs (index.html + js +
    // css). Serving the dir that holds index.html is robust and avoids running a stray /
    // broken generated server file when a perfectly good static entry exists (the cause of
    // a "Run the app" 404 when the model emitted both an index.html and a server.js).
    for dir in ["app", "app/public", "public", "."] {
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
    // No static entry — fall back to an actual server file (reads PORT), e.g. an API app.
    for (f, runner) in candidates {
        let p = repo.join(f);
        if p.exists() && looks_like_server(&p) {
            return Some((vec![runner.into(), f.into()], repo.to_path_buf(), f.into()));
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
fn fail_run(state: &Arc<State>, run_id: &str, spec_ref: &str, e: &anyhow::Error) {
    runstate::append_event(
        &state.repo,
        run_id,
        &runstate::Event {
            seq: 9999,
            ts: crate::core::timeutil::iso_now(),
            icon: "❌".into(),
            msg: format!("run failed: {e}"),
        },
    );
    runstate::write_status(
        &state.repo,
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

fn seed_status(state: &Arc<State>, run_id: &str, spec_ref: &str, status: &str) {
    runstate::write_status(
        &state.repo,
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
fn run_view(id: &str, state: &Arc<State>) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    if let Ok(rec) = runstate::load_run(&state.repo, id) {
        return Ok(json_resp(200, &json!(rec)));
    }
    match runstate::read_status(&state.repo, id) {
        Some(st) => Ok(json_resp(200, &json!(st))),
        None => Ok(json_resp(404, &json!({ "error": "no such run" }))),
    }
}

fn reputation_view(state: &Arc<State>) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let mut atts = vec![];
    for rec in runstate::list_runs(&state.repo)? {
        if let Ok(text) = std::fs::read_to_string(rec.attestation_path(&state.repo)) {
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

fn parse_since(query: &str) -> u64 {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("since="))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
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
    Response::from_data(s.as_bytes().to_vec()).with_header(ctype(ct))
}

fn ctype(ct: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap()
}
