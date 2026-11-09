//! `traindle-explorer serve`: a small local web UI.
//!
//! Serves the page from `web/` (embedded at build time; a `web/` folder next to the
//! working directory overrides it, for editing without rebuilding), runs one solve
//! job at a time on a background thread, and stores every run under `runs/<id>/`.
//! The page polls `/api/status` for progress. Verification runs in the browser with
//! the shared checks (web/verify-core.js, using mathjs as the game's evaluator) and posts
//! its report back to `/api/runs/<id>/verify`.

use crate::config::{SolverConfig, ALL_FEATURES};
use crate::driver::{all_numbers, run};
use crate::progress::Progress;
use crate::runs::{build_outputs, new_run_id, now_stamp, save_run, slug, valid_id, write_json, SweepInfo};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_http::{Header, Method, Request, Response, Server};

const MATHJS_CDN: &str = "https://cdn.jsdelivr.net/npm/mathjs@15.2.0/lib/browser/math.js";
const MATHJS_LOCAL: &str = "verify/node_modules/mathjs/lib/browser/math.js";

const STATIC: [(&str, &str, &str); 3] = [
    ("index.html", "text/html; charset=utf-8", include_str!("../web/index.html")),
    ("verify-core.js", "text/javascript; charset=utf-8", include_str!("../web/verify-core.js")),
    ("verify-worker.js", "text/javascript; charset=utf-8", include_str!("../web/verify-worker.js")),
];

/// Most targets in one sweep.
const MAX_SWEEP: usize = 200;

/// One solve job: a single run, or a sweep (one run per target, in order).
struct Job {
    /// run id for a single run, sweep id for a sweep
    id: String,
    name: String,
    /// the run currently being solved (= id for a single run)
    run_id: Mutex<String>,
    /// (sweep id, target count) for sweeps
    sweep: Option<(String, usize)>,
    targets: Vec<String>,
    /// 0-based index of the target being solved
    index: AtomicUsize,
    progress: Arc<Progress>,
    /// "running" | "done" | "cancelled" | "error"
    state: Mutex<(String, Option<String>)>,
    started: Instant,
    elapsed: Mutex<Option<f64>>,
}

struct App {
    runs_dir: PathBuf,
    job: Mutex<Option<Arc<Job>>>,
    /// per run: (puzzle number, byte offset) of each line of its alternatives file
    alt_index: Mutex<Vec<(String, Arc<AltIndex>)>>,
}

struct AltIndex {
    path: PathBuf,
    lines: Vec<(u32, u64)>,
}

/// Index a `solutions.alts.jsonl` file: the puzzle number and offset of every line.
fn index_alts(path: &Path) -> std::io::Result<AltIndex> {
    let bytes = std::fs::read(path)?;
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            // a line starts with ["<digits>"
            let num: u32 = bytes[start..i].iter().skip(2).take_while(|c| c.is_ascii_digit()).fold(0, |a, &c| a * 10 + (c - b'0') as u32);
            lines.push((num, start as u64));
            start = i + 1;
        }
    }
    Ok(AltIndex { path: path.to_path_buf(), lines })
}

/// The alternatives of one puzzle as a JSON array of [cost, expression] ("[]" if none).
fn alts_for(app: &App, id: &str, number: &str) -> std::io::Result<String> {
    use std::io::{BufRead, Seek, SeekFrom};
    let path = crate::runs::alts_path(&app.runs_dir.join(id).join("solutions.json"));
    let idx = {
        let mut cache = app.alt_index.lock().unwrap();
        match cache.iter().find(|(k, _)| k == id) {
            Some((_, ix)) => ix.clone(),
            None => {
                let ix = Arc::new(index_alts(&path)?);
                cache.insert(0, (id.to_string(), ix.clone()));
                cache.truncate(4); // a few recent runs (A, B, ...)
                ix
            }
        }
    };
    let n: u32 = number.parse().map_err(|_| std::io::Error::other("bad number"))?;
    let Ok(k) = idx.lines.binary_search_by_key(&n, |l| l.0) else { return Ok("[]".into()) };
    let mut f = std::io::BufReader::new(std::fs::File::open(&idx.path)?);
    f.seek(SeekFrom::Start(idx.lines[k].1))?;
    let mut line = String::new();
    f.read_line(&mut line)?;
    let v: Value = serde_json::from_str(line.trim()).map_err(std::io::Error::other)?;
    Ok(v[1].to_string())
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn json_resp(v: &Value, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(v.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("Cache-Control", "no-store"))
}

fn err(msg: &str, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    json_resp(&json!({ "error": msg }), status)
}

pub fn serve(port: u16, runs_dir: PathBuf, open: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(&runs_dir)?;
    let server = Server::http(("127.0.0.1", port)).map_err(|e| std::io::Error::other(e.to_string()))?;
    let url = format!("http://127.0.0.1:{port}/");
    eprintln!("Traindle Explorer at {url}  (runs saved in {})", runs_dir.display());
    if Path::new(MATHJS_LOCAL).exists() {
        eprintln!("verification uses local mathjs ({MATHJS_LOCAL})");
    } else {
        eprintln!("verification loads mathjs 15.2.0 from the CDN (run `npm install` in verify/ to use a local copy)");
    }
    if open {
        open_browser(&url);
    }
    let app = Arc::new(App { runs_dir, job: Mutex::new(None), alt_index: Mutex::new(Vec::new()) });
    for req in server.incoming_requests() {
        let app = app.clone();
        // requests are cheap; one thread each keeps a slow file read from blocking polls
        std::thread::spawn(move || handle(&app, req));
    }
    Ok(())
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if r.is_err() {
        eprintln!("open {url} in your browser");
    }
}

fn handle(app: &App, mut req: Request) {
    let method = req.method().clone();
    let url = req.url().split('?').next().unwrap_or("/").to_string();
    let mut body = String::new();
    if method == Method::Post {
        let _ = req.as_reader().take(64 << 20).read_to_string(&mut body);
    }
    let parts: Vec<&str> = url.trim_matches('/').split('/').collect();
    let _ = match (method, parts.as_slice()) {
        (Method::Get, [""]) => static_file(req, "index.html"),
        (Method::Get, [name]) if STATIC.iter().any(|s| s.0 == *name) => static_file(req, name),
        (Method::Get, ["vendor", "math.js"]) => {
            if let Ok(f) = std::fs::File::open(MATHJS_LOCAL) {
                req.respond(Response::from_file(f).with_header(header("Content-Type", "text/javascript")))
            } else {
                req.respond(Response::empty(302).with_header(header("Location", MATHJS_CDN)))
            }
        }
        (Method::Get, ["api", "defaults"]) => req.respond(json_resp(&defaults(), 200)),
        (Method::Post, ["api", "run"]) => {
            let r = start_job(app, &body, false);
            req.respond(r)
        }
        (Method::Post, ["api", "sweep"]) => {
            let r = start_job(app, &body, true);
            req.respond(r)
        }
        (Method::Get, ["api", "status"]) => req.respond(json_resp(&status(app), 200)),
        (Method::Post, ["api", "cancel"]) => {
            if let Some(job) = app.job.lock().unwrap().as_ref() {
                job.progress.cancel();
            }
            req.respond(json_resp(&json!({ "ok": true }), 200))
        }
        (Method::Get, ["api", "runs"]) => req.respond(json_resp(&list_runs(&app.runs_dir), 200)),
        (Method::Get, ["api", "runs", id, "alts", number]) => {
            let ok = valid_id(id) && !number.is_empty() && number.len() <= 6 && number.bytes().all(|c| c.is_ascii_digit());
            let r = if !ok {
                err("not found", 404)
            } else {
                match alts_for(app, id, number) {
                    Ok(s) => Response::from_string(s)
                        .with_header(header("Content-Type", "application/json"))
                        .with_header(header("Cache-Control", "no-store")),
                    Err(_) => err("no alternatives file for this run", 404),
                }
            };
            req.respond(r)
        }
        (Method::Get, ["api", "runs", id, file]) => {
            let allowed = ["config.json", "solutions.json", "summary.json", "verify.json"];
            if !valid_id(id) || !allowed.contains(file) {
                req.respond(err("not found", 404))
            } else {
                match std::fs::File::open(app.runs_dir.join(id).join(file)) {
                    Ok(f) => req.respond(
                        Response::from_file(f)
                            .with_header(header("Content-Type", "application/json"))
                            .with_header(header("Cache-Control", "no-store")),
                    ),
                    Err(_) => req.respond(err("not found", 404)),
                }
            }
        }
        (Method::Post, ["api", "runs", id, "verify"]) => {
            let r = if !valid_id(id) || !app.runs_dir.join(id).is_dir() {
                err("no such run", 404)
            } else {
                match serde_json::from_str::<Value>(&body) {
                    Ok(v) => match write_json(&app.runs_dir.join(id).join("verify.json"), &v, true) {
                        Ok(_) => json_resp(&json!({ "ok": true }), 200),
                        Err(e) => err(&e.to_string(), 500),
                    },
                    Err(e) => err(&format!("bad json: {e}"), 400),
                }
            };
            req.respond(r)
        }
        (Method::Delete, ["api", "runs", id]) => {
            let running = app.job.lock().unwrap().as_ref().is_some_and(|j| *j.run_id.lock().unwrap() == *id && j.state.lock().unwrap().0 == "running");
            let r = if !valid_id(id) || running {
                err("cannot delete this run", 400)
            } else {
                match std::fs::remove_dir_all(app.runs_dir.join(id)) {
                    Ok(_) => json_resp(&json!({ "ok": true }), 200),
                    Err(e) => err(&e.to_string(), 404),
                }
            };
            req.respond(r)
        }
        _ => req.respond(err("not found", 404)),
    };
}

fn static_file(req: Request, name: &str) -> std::io::Result<()> {
    let (_, ctype, embedded) = STATIC.iter().find(|s| s.0 == name).unwrap();
    let disk = Path::new("web").join(name);
    let body = std::fs::read_to_string(&disk).unwrap_or_else(|_| embedded.to_string());
    req.respond(
        Response::from_string(body)
            .with_header(header("Content-Type", ctype))
            .with_header(header("Cache-Control", "no-store")),
    )
}

fn defaults() -> Value {
    let mut cfg = SolverConfig::default();
    cfg.normalize().expect("default config is valid");
    let features: Vec<Value> = ALL_FEATURES
        .iter()
        .map(|f| {
            json!({
                "id": serde_json::to_value(f).unwrap(),
                "label": f.label(),
                "needs_brackets": f.needs_brackets(),
                "weight_key": f.weight_key(),
            })
        })
        .collect();
    json!({ "config": cfg, "features": features, "max_digits": 6 })
}

/// Start a run (`{config}`) or a sweep (`{config, targets: [..]}`).
fn start_job(app: &App, body: &str, sweep: bool) -> Response<std::io::Cursor<Vec<u8>>> {
    let (mut sc, targets): (SolverConfig, Option<Vec<String>>) = if sweep {
        #[derive(serde::Deserialize)]
        struct SweepReq {
            config: SolverConfig,
            targets: Vec<String>,
        }
        match serde_json::from_str::<SweepReq>(body) {
            Ok(r) => (r.config, Some(r.targets)),
            Err(e) => return err(&format!("bad sweep: {e}"), 400),
        }
    } else {
        match serde_json::from_str(body) {
            Ok(c) => (c, None),
            Err(e) => return err(&format!("bad config: {e}"), 400),
        }
    };
    if let Err(e) = sc.normalize() {
        return err(&e, 400);
    }
    // one validated config per target
    let configs: Vec<SolverConfig> = match &targets {
        None => vec![sc.clone()],
        Some(ts) => {
            if ts.is_empty() || ts.len() > MAX_SWEEP {
                return err(&format!("a sweep needs 1-{MAX_SWEEP} targets"), 400);
            }
            let mut v = Vec::new();
            for t in ts {
                let mut c = sc.clone();
                c.target = t.clone();
                c.name = format!("{} → {}", sc.name, t.trim()).trim_start().to_string();
                if let Err(e) = c.normalize() {
                    return err(&format!("target {t:?}: {e}"), 400);
                }
                v.push(c);
            }
            v
        }
    };
    let mut slot = app.job.lock().unwrap();
    if slot.as_ref().is_some_and(|j| j.state.lock().unwrap().0 == "running") {
        return err("a run is already in progress", 409);
    }
    let first_id = new_run_id(&app.runs_dir, &configs[0].name);
    let sweep_id = targets.as_ref().map(|_| {
        let compact = now_stamp().1;
        let s = slug(&sc.name);
        if s.is_empty() { format!("sweep-{compact}") } else { format!("sweep-{compact}-{s}") }
    });
    let job = Arc::new(Job {
        id: sweep_id.clone().unwrap_or_else(|| first_id.clone()),
        name: sc.name.clone(),
        run_id: Mutex::new(first_id.clone()),
        sweep: sweep_id.map(|id| (id, configs.len())),
        targets: configs.iter().map(|c| c.target.clone()).collect(),
        index: AtomicUsize::new(0),
        progress: Arc::new(Progress::default()),
        state: Mutex::new(("running".into(), None)),
        started: Instant::now(),
        elapsed: Mutex::new(None),
    });
    *slot = Some(job.clone());
    let runs_dir = app.runs_dir.clone();
    let reply = json!({ "id": job.id });
    std::thread::spawn(move || {
        let mut state = ("done".to_string(), None);
        for (i, cfg) in configs.iter().enumerate() {
            if job.progress.cancelled() {
                break;
            }
            let id = if i == 0 { first_id.clone() } else { new_run_id(&runs_dir, &cfg.name) };
            *job.run_id.lock().unwrap() = id.clone();
            job.index.store(i, Ordering::Relaxed);
            if job.sweep.is_some() {
                job.progress.log(format!("── target {} ({}/{})", cfg.target, i + 1, configs.len()));
            }
            let numbers = all_numbers(cfg.digits);
            let t0 = Instant::now();
            let res = run(&numbers, cfg, &job.progress, false);
            let elapsed = t0.elapsed().as_secs_f64();
            let info = job.sweep.as_ref().map(|(sid, n)| SweepInfo { id: sid.clone(), name: job.name.clone(), index: i, count: *n });
            let (entries, summary) = build_outputs(&id, &numbers, cfg, &res, elapsed, info);
            match save_run(&runs_dir.join(&id), &entries, &summary, &numbers, &res) {
                Ok(_) if res.cancelled => state = ("cancelled".to_string(), None),
                Ok(_) => {}
                Err(e) => {
                    state = ("error".to_string(), Some(format!("could not save run: {e}")));
                    break;
                }
            }
        }
        *job.elapsed.lock().unwrap() = Some(job.started.elapsed().as_secs_f64());
        *job.state.lock().unwrap() = state;
    });
    json_resp(&reply, 200)
}

fn status(app: &App) -> Value {
    let slot = app.job.lock().unwrap();
    let Some(job) = slot.as_ref() else {
        return json!({ "state": "idle" });
    };
    let (state, error) = job.state.lock().unwrap().clone();
    let elapsed = job.elapsed.lock().unwrap().unwrap_or_else(|| job.started.elapsed().as_secs_f64());
    json!({
        "state": state,
        "error": error,
        "id": job.id,
        "run_id": *job.run_id.lock().unwrap(),
        "name": job.name,
        "sweep": job.sweep.as_ref().map(|(id, n)| {
            let i = job.index.load(Ordering::Relaxed);
            json!({ "id": id, "i": i, "n": n, "target": job.targets.get(i) })
        }),
        "elapsed_s": elapsed,
        "progress": job.progress.snapshot(),
    })
}

fn list_runs(dir: &Path) -> Value {
    let mut runs: Vec<Value> = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let Ok(text) = std::fs::read_to_string(p.join("summary.json")) else { continue };
            let Ok(s) = serde_json::from_str::<Value>(&text) else { continue };
            let verify = std::fs::read_to_string(p.join("verify.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(|v| json!({ "ok": v["ok"], "failures": v["failure_count"], "checked": v["checked"] }));
            runs.push(json!({
                "id": s["id"], "name": s["name"], "created": s["created"], "digits": s["digits"],
                "target": s["target"], "total": s["total"], "solved": s["solved"],
                "tier_names": s["tier_names"], "tier_counts": s["tier_counts"],
                "elapsed_s": s["elapsed_s"], "cancelled": s["cancelled"], "verify": verify,
                "sweep": s["sweep"], "has_rarity": s["config"]["alt_slack"].is_u64(),
                "alt_slack": s["config"]["alt_slack"],
            }));
        }
    }
    runs.sort_by(|a, b| b["id"].as_str().cmp(&a["id"].as_str()));
    json!(runs)
}
