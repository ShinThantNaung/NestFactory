//! Dev bridge. Serves `Backend::dispatch` at `POST /invoke` and events as Server-Sent
//! Events at `GET /events`, on 127.0.0.1 only, so the UI can be developed and tested in a
//! browser against the real Rust backend.
//!
//! Every request needs the per-run token printed at startup, and browser requests must come
//! from the Vite dev server origin, so other local pages can't drive the backend.
//!
//! Usage: cargo run -p nestfactory-bridge [-- --port 1421 --open <project>]

use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nestfactory_service::{Backend, EventSink};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

const ALLOWED_ORIGINS: &[&str] = &["http://localhost:1420", "http://127.0.0.1:1420"];

#[derive(Default)]
struct Sse {
    clients: Mutex<Vec<Sender<String>>>,
}

impl EventSink for Sse {
    fn emit(&self, event: &str, payload: Value) {
        let msg = format!("event: {event}\ndata: {}\n\n", payload);
        self.clients.lock().unwrap().retain(|c| c.send(msg.clone()).is_ok());
    }
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn origin(req: &Request) -> Option<String> {
    req.headers().iter().find(|h| h.field.equiv("Origin")).map(|h| h.value.as_str().to_string())
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(String::from))
}

fn cors(resp: Response<std::io::Cursor<Vec<u8>>>, origin: &Option<String>) -> Response<std::io::Cursor<Vec<u8>>> {
    match origin {
        Some(o) if ALLOWED_ORIGINS.contains(&o.as_str()) => resp
            .with_header(header("Access-Control-Allow-Origin", o))
            .with_header(header("Access-Control-Allow-Headers", "content-type, x-bridge-token"))
            .with_header(header("Access-Control-Allow-Methods", "POST, GET, OPTIONS"))
            .with_header(header("Vary", "Origin")),
        _ => resp,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.iter().position(|a| a == "--port").and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()).unwrap_or(1421);
    let open = args.iter().position(|a| a == "--open").and_then(|i| args.get(i + 1)).cloned();
    let token = args
        .iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var("NESTFACTORY_BRIDGE_TOKEN").ok())
        .unwrap_or_else(|| nestfactory_core::wizard::dev_secret()[..24].to_string());

    let data_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("dev.nestfactory.app");
    let sse = Arc::new(Sse::default());
    let backend = Backend::new(data_dir, sse.clone());
    if let Some(p) = open {
        match backend.open_project(std::path::Path::new(&p)) {
            Ok(m) => eprintln!("opened {} ({} modules, {:.0} ms)", m.name, m.modules.len(), m.scan_ms),
            Err(e) => eprintln!("could not open {p}: {e:#}"),
        }
    }
    let _ = std::fs::write(".bridge-token", &token);
    let server = Server::http(("127.0.0.1", port)).expect("bind bridge port");
    println!("NestFactory bridge on http://127.0.0.1:{port}");
    println!("Open the UI at http://localhost:1420/?bridge={port}&token={token}");

    for req in server.incoming_requests() {
        let backend = backend.clone();
        let sse = sse.clone();
        let token = token.clone();
        std::thread::spawn(move || handle(req, backend, sse, &token));
    }
}

fn handle(mut req: Request, backend: Backend, sse: Arc<Sse>, token: &str) {
    let origin = origin(&req);
    if let Some(o) = &origin {
        if !ALLOWED_ORIGINS.contains(&o.as_str()) {
            let _ = req.respond(Response::from_string("forbidden origin").with_status_code(403));
            return;
        }
    }
    if *req.method() == Method::Options {
        let _ = req.respond(cors(Response::from_data(vec![]).with_status_code(204), &origin));
        return;
    }
    let url = req.url().to_string();
    let given = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("x-bridge-token"))
        .map(|h| h.value.as_str().to_string())
        .or_else(|| query_param(&url, "token"));
    if given.as_deref() != Some(token) {
        let _ = req.respond(cors(Response::from_string("bad token").with_status_code(401), &origin));
        return;
    }
    let path = url.split('?').next().unwrap_or("");
    match (req.method().clone(), path) {
        (Method::Post, "/invoke") => {
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                let _ = req.respond(Response::from_string("bad body").with_status_code(400));
                return;
            }
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let cmd = v.get("cmd").and_then(Value::as_str).unwrap_or("").to_string();
            let args = v.get("args").cloned().unwrap_or(json!({}));
            let out = match backend.dispatch(&cmd, args) {
                Ok(data) => json!({ "ok": true, "data": data }),
                Err(e) => json!({ "ok": false, "error": e }),
            };
            let resp = Response::from_string(out.to_string()).with_header(header("Content-Type", "application/json"));
            let _ = req.respond(cors(resp, &origin));
        }
        (Method::Get, "/events") => stream_events(req, sse, origin),
        _ => {
            let _ = req.respond(cors(Response::from_string("not found").with_status_code(404), &origin));
        }
    }
}

fn stream_events(req: Request, sse: Arc<Sse>, origin: Option<String>) {
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    sse.clients.lock().unwrap().push(tx);
    let mut w = req.into_writer();
    let allow = origin.filter(|o| ALLOWED_ORIGINS.contains(&o.as_str())).map(|o| format!("Access-Control-Allow-Origin: {o}\r\n")).unwrap_or_default();
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n{allow}\r\n: connected\n\n");
    if w.write_all(head.as_bytes()).and_then(|_| w.flush()).is_err() {
        return;
    }
    loop {
        let msg = match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(m) => m,
            Err(mpsc::RecvTimeoutError::Timeout) => ": ping\n\n".to_string(),
            Err(_) => return,
        };
        if w.write_all(msg.as_bytes()).and_then(|_| w.flush()).is_err() {
            return;
        }
    }
}
