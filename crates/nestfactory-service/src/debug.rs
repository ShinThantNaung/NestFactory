//! Debugger: a small Chrome DevTools Protocol client. It starts the app with
//! `start:debug` (or Jest with `--inspect-brk`), attaches over WebSocket, maps TypeScript
//! breakpoints to the compiled JavaScript through source maps, and reports pauses with the
//! call stack mapped back to the `.ts` files. Watch-mode restarts reattach automatically.

use std::collections::HashMap;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use nestfactory_core::model::PackageManager;
use nestfactory_core::pm;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sourcemap::SourceMap;
use ts_rs::TS;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::process::{command, ProcessManager};
use crate::EventSink;

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Breakpoint {
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub ty: String,
    pub object_id: Option<String>,
}

enum Out {
    Command(String),
    SyncBreakpoints,
    Properties(String, Sender<Value>),
}

struct Script {
    id: String,
    map: Option<SourceMap>,
    /// Normalized absolute source paths, indexed like the map's sources.
    sources: Vec<String>,
    generated: String,
}

pub struct Session {
    tx: Sender<Out>,
    breakpoints: Arc<Mutex<Vec<Breakpoint>>>,
    stopped: Arc<AtomicBool>,
    procs: Arc<ProcessManager>,
}

const PROC_ID: &str = "debug";

fn norm(p: &str) -> String {
    let p = p.strip_prefix("file:///").or_else(|| p.strip_prefix("file://")).unwrap_or(p);
    let p = p.replace("%20", " ").replace('\\', "/");
    let p = if cfg!(windows) { p.to_lowercase() } else { p };
    nestfactory_core::project::normalize_path(&p)
}

impl Session {
    pub fn start(
        root: PathBuf,
        pmgr: PackageManager,
        mode: &str,
        file: Option<String>,
        breakpoints: Vec<Breakpoint>,
        procs: Arc<ProcessManager>,
        sink: Arc<dyn EventSink>,
    ) -> Result<Session> {
        let port = if mode == "test" { 9230 } else { 9229 };
        let cmd = if mode == "test" {
            let runner = crate::test_runner(&root).ok_or_else(|| anyhow!("Neither Jest nor Vitest is installed in this project"))?;
            let bin = runner.bin.to_string_lossy().to_string();
            let mut args = if runner.vitest {
                // Vitest opens the inspector in its test worker.
                vec![bin, "run".to_string(), format!("--inspect-brk=127.0.0.1:{port}"), "--no-file-parallelism".to_string()]
            } else {
                vec![format!("--inspect-brk=127.0.0.1:{port}"), bin, "--runInBand".to_string(), "--no-coverage".to_string()]
            };
            if let Some(f) = &file {
                args.push(f.clone());
            }
            command("node", &args, &root, &[("FORCE_COLOR".into(), "1".into())])?
        } else {
            let line = pm::run_script(pmgr, "start:debug");
            command(&line[0], &line[1..], &root, &[("FORCE_COLOR".into(), "1".into())])?
        };
        procs.spawn(PROC_ID, cmd)?;
        let (tx, rx) = mpsc::channel();
        let bps = Arc::new(Mutex::new(breakpoints));
        let stopped = Arc::new(AtomicBool::new(false));
        let worker = Worker { root, port, sink: sink.clone(), breakpoints: bps.clone(), stopped: stopped.clone(), procs: procs.clone(), scripts: vec![], bp_ids: vec![], pending: HashMap::new(), next_id: AtomicU64::new(1) };
        std::thread::spawn(move || worker.run(rx));
        sink.emit("debug-state", json!({ "state": "starting", "detail": if mode == "test" { "Starting Jest with the inspector" } else { "Starting the app in debug mode" } }));
        Ok(Session { tx, breakpoints: bps, stopped, procs })
    }

    pub fn set_breakpoints(&self, bps: Vec<Breakpoint>) {
        *self.breakpoints.lock().unwrap() = bps;
        let _ = self.tx.send(Out::SyncBreakpoints);
    }

    pub fn command(&self, c: &str) -> Result<()> {
        let method = match c {
            "resume" => "Debugger.resume",
            "stepOver" => "Debugger.stepOver",
            "stepInto" => "Debugger.stepInto",
            "stepOut" => "Debugger.stepOut",
            "pause" => "Debugger.pause",
            _ => return Err(anyhow!("unknown debug command {c}")),
        };
        self.tx.send(Out::Command(method.into())).map_err(|_| anyhow!("The debug session has ended"))
    }

    pub fn properties(&self, object_id: &str) -> Result<Vec<Variable>> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(Out::Properties(object_id.to_string(), tx)).map_err(|_| anyhow!("The debug session has ended"))?;
        let v = rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("The debugger didn't answer"))?;
        Ok(variables(&v))
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.procs.kill(PROC_ID);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

fn variables(result: &Value) -> Vec<Variable> {
    let mut out = vec![];
    for p in result.pointer("/result").and_then(Value::as_array).into_iter().flatten() {
        let name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        if name == "__proto__" {
            continue;
        }
        let Some(v) = p.get("value") else { continue };
        let ty = v.get("subtype").or_else(|| v.get("type")).and_then(Value::as_str).unwrap_or("").to_string();
        let value = match v.get("type").and_then(Value::as_str) {
            Some("string") => format!("\"{}\"", v.get("value").and_then(Value::as_str).unwrap_or("")),
            Some("undefined") => "undefined".into(),
            _ => v.get("description").and_then(Value::as_str).map(String::from).or_else(|| v.get("value").map(|x| x.to_string())).unwrap_or_default(),
        };
        let object_id = v.get("objectId").and_then(Value::as_str).map(String::from);
        out.push(Variable { name, value, ty, object_id });
    }
    out
}

struct Worker {
    root: PathBuf,
    port: u16,
    sink: Arc<dyn EventSink>,
    breakpoints: Arc<Mutex<Vec<Breakpoint>>>,
    stopped: Arc<AtomicBool>,
    procs: Arc<ProcessManager>,
    scripts: Vec<Script>,
    bp_ids: Vec<String>,
    pending: HashMap<u64, Sender<Value>>,
    next_id: AtomicU64,
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

impl Worker {
    fn emit(&self, e: &str, v: Value) {
        self.sink.emit(e, v);
    }

    fn run(mut self, rx: Receiver<Out>) {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(500)).build();
        while !self.stopped.load(Ordering::SeqCst) {
            if !self.procs.is_running(PROC_ID) {
                break;
            }
            let url = format!("http://127.0.0.1:{}/json/list", self.port);
            let ws_url = agent
                .get(&url)
                .call()
                .ok()
                .and_then(|r| r.into_json::<Value>().ok())
                .and_then(|v| v.as_array().and_then(|a| a.first().cloned()))
                .and_then(|t| t.get("webSocketDebuggerUrl").and_then(Value::as_str).map(String::from));
            let Some(ws_url) = ws_url else {
                std::thread::sleep(Duration::from_millis(300));
                continue;
            };
            match tungstenite::connect(ws_url.as_str()) {
                Ok((ws, _)) => {
                    self.emit("debug-state", json!({ "state": "running", "detail": format!("Inspector attached · port {}", self.port) }));
                    self.session(ws, &rx);
                    self.emit("debug-state", json!({ "state": "reconnecting", "detail": "Waiting for the app to restart" }));
                    self.scripts.clear();
                    self.bp_ids.clear();
                }
                Err(_) => std::thread::sleep(Duration::from_millis(300)),
            }
        }
        self.emit("debug-state", json!({ "state": "ended", "detail": "" }));
    }

    fn send(&self, ws: &mut Ws, method: &str, params: Value) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _ = ws.send(Message::Text(json!({ "id": id, "method": method, "params": params }).to_string()));
        id
    }

    fn session(&mut self, mut ws: Ws, rx: &Receiver<Out>) {
        if let MaybeTlsStream::Plain(s) = ws.get_mut() {
            let _ = s.set_read_timeout(Some(Duration::from_millis(40)));
        }
        self.send(&mut ws, "Runtime.enable", json!({}));
        self.send(&mut ws, "Debugger.enable", json!({}));
        self.send(&mut ws, "Debugger.setBreakpointsActive", json!({ "active": true }));
        self.send(&mut ws, "Runtime.runIfWaitingForDebugger", json!({}));
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                let _ = ws.close(None);
                return;
            }
            while let Ok(out) = rx.try_recv() {
                match out {
                    Out::Command(m) => {
                        self.send(&mut ws, &m, json!({}));
                    }
                    Out::SyncBreakpoints => self.sync_breakpoints(&mut ws),
                    Out::Properties(object_id, reply) => {
                        let id = self.send(&mut ws, "Runtime.getProperties", json!({ "objectId": object_id, "ownProperties": true, "generatePreview": true }));
                        self.pending.insert(id, reply);
                    }
                }
            }
            match ws.read() {
                Ok(Message::Text(t)) => {
                    let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                    self.on_message(&mut ws, v);
                }
                Ok(Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return,
            }
        }
    }

    fn on_message(&mut self, ws: &mut Ws, v: Value) {
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            if let Some(reply) = self.pending.remove(&id) {
                let _ = reply.send(v.get("result").cloned().unwrap_or(Value::Null));
            }
            return;
        }
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        match v.get("method").and_then(Value::as_str) {
            Some("Debugger.scriptParsed") => {
                let url = params.get("url").and_then(Value::as_str).unwrap_or("");
                if url.is_empty() || url.contains("node_modules") || url.starts_with("node:") {
                    return;
                }
                let generated = norm(url);
                if !generated.starts_with(&norm(&self.root.to_string_lossy())) {
                    return;
                }
                let id = params.get("scriptId").and_then(Value::as_str).unwrap_or("").to_string();
                let map_url = params.get("sourceMapURL").and_then(Value::as_str).unwrap_or("");
                let (map, sources) = load_map(url, map_url);
                self.scripts.push(Script { id, map, sources, generated });
                let idx = self.scripts.len() - 1;
                self.set_for_script(ws, idx);
            }
            Some("Debugger.paused") => {
                let frames: Vec<Value> = params
                    .get("callFrames")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|f| self.frame(f))
                    .collect();
                let reason = params.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
                self.emit("debug-paused", json!({ "reason": reason, "frames": frames }));
                self.emit("debug-state", json!({ "state": "paused", "detail": if reason == "other" { "Paused on breakpoint".to_string() } else { format!("Paused · {reason}") } }));
            }
            Some("Debugger.resumed") => {
                self.emit("debug-resumed", json!({}));
                self.emit("debug-state", json!({ "state": "running", "detail": format!("Inspector attached · port {}", self.port) }));
            }
            _ => {}
        }
    }

    fn frame(&self, f: &Value) -> Value {
        let script_id = f.pointer("/location/scriptId").and_then(Value::as_str).unwrap_or("");
        let line = f.pointer("/location/lineNumber").and_then(Value::as_u64).unwrap_or(0) as u32;
        let col = f.pointer("/location/columnNumber").and_then(Value::as_u64).unwrap_or(0) as u32;
        let url = f.get("url").and_then(Value::as_str).unwrap_or("");
        let root = norm(&self.root.to_string_lossy());
        let (file, line_out, col_out) = match self.scripts.iter().find(|s| s.id == script_id) {
            Some(s) => match s.map.as_ref().and_then(|m| m.lookup_token(line, col)) {
                Some(tok) => {
                    let src = tok.get_src_id() as usize;
                    let file = s.sources.get(src).cloned().unwrap_or_else(|| s.generated.clone());
                    (file, tok.get_src_line() + 1, tok.get_src_col() + 1)
                }
                None => (s.generated.clone(), line + 1, col + 1),
            },
            None => (norm(url), line + 1, col + 1),
        };
        let rel = file.strip_prefix(&format!("{root}/")).map(String::from);
        let this_scope = f
            .pointer("/this/objectId")
            .and_then(Value::as_str)
            .map(|id| json!({ "type": "this", "name": f.pointer("/this/className").and_then(Value::as_str).unwrap_or(""), "objectId": id }));
        let scopes: Vec<Value> = this_scope.into_iter().chain(f
            .get("scopeChain")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|s| s.get("type").and_then(Value::as_str) != Some("global"))
            .map(|s| {
                json!({
                    "type": s.get("type").and_then(Value::as_str).unwrap_or(""),
                    "name": s.get("name").and_then(Value::as_str).unwrap_or(""),
                    "objectId": s.pointer("/object/objectId").and_then(Value::as_str).unwrap_or(""),
                })
            }))
            .collect();
        json!({
            "function": f.get("functionName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("(anonymous)"),
            "file": rel,
            "line": line_out,
            "column": col_out,
            "external": rel.is_none(),
            "scopes": scopes,
        })
    }

    fn sync_breakpoints(&mut self, ws: &mut Ws) {
        for id in std::mem::take(&mut self.bp_ids) {
            self.send(ws, "Debugger.removeBreakpoint", json!({ "breakpointId": id }));
        }
        for i in 0..self.scripts.len() {
            self.set_for_script(ws, i);
        }
    }

    fn set_for_script(&mut self, ws: &mut Ws, idx: usize) {
        let root = norm(&self.root.to_string_lossy());
        let bps = self.breakpoints.lock().unwrap().clone();
        let mut ids = vec![];
        {
            let s = &self.scripts[idx];
            for bp in &bps {
                let target = norm(&format!("{root}/{}", bp.file));
                let Some(src_idx) = s.sources.iter().position(|x| *x == target) else {
                    // A script without a map is its own source.
                    if s.map.is_none() && s.generated == target {
                        ids.push((s.id.clone(), bp.line - 1, 0));
                    }
                    continue;
                };
                if let Some(m) = &s.map {
                    if let Some((l, c)) = generated_location(m, src_idx as u32, bp.line - 1) {
                        ids.push((s.id.clone(), l, c));
                    }
                }
            }
        }
        for (script, line, col) in ids {
            let id = self.send(ws, "Debugger.setBreakpoint", json!({ "location": { "scriptId": script, "lineNumber": line, "columnNumber": col } }));
            // The breakpoint id arrives in the response; track it for removal.
            let (tx, rx) = mpsc::channel();
            self.pending.insert(id, tx);
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
                if let Ok(v) = rx.try_recv() {
                    if let Some(b) = v.get("breakpointId").and_then(Value::as_str) {
                        self.bp_ids.push(b.to_string());
                    }
                    break;
                }
                match ws.read() {
                    Ok(Message::Text(t)) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&t) {
                            self.on_message(ws, v);
                        }
                    }
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    _ => break,
                }
            }
        }
    }
}

/// The earliest generated position for an original line (or the next few lines).
fn generated_location(m: &SourceMap, src: u32, line: u32) -> Option<(u32, u32)> {
    for l in line..line + 4 {
        let best = m.tokens().filter(|t| t.get_src_id() == src && t.get_src_line() == l).map(|t| (t.get_dst_line(), t.get_dst_col())).min();
        if best.is_some() {
            return best;
        }
    }
    None
}

fn load_map(script_url: &str, map_url: &str) -> (Option<SourceMap>, Vec<String>) {
    if map_url.is_empty() {
        return (None, vec![]);
    }
    let script_path = PathBuf::from(script_url.strip_prefix("file:///").or_else(|| script_url.strip_prefix("file://")).unwrap_or(script_url).replace("%20", " "));
    let (bytes, base): (Vec<u8>, PathBuf) = if let Some(data) = map_url.strip_prefix("data:") {
        let Some(i) = data.find("base64,") else { return (None, vec![]) };
        (base64_decode(&data[i + 7..]), script_path.parent().map(Path::to_path_buf).unwrap_or_default())
    } else {
        let map_path = script_path.parent().map(|p| p.join(map_url)).unwrap_or_else(|| PathBuf::from(map_url));
        match std::fs::read(&map_path) {
            Ok(b) => (b, map_path.parent().map(Path::to_path_buf).unwrap_or_default()),
            Err(_) => return (None, vec![]),
        }
    };
    let Ok(map) = SourceMap::from_slice(&bytes) else { return (None, vec![]) };
    let source_root = map.get_source_root().unwrap_or("").to_string();
    let sources = map
        .sources()
        .map(|s| {
            let s = s.strip_prefix("file:///").unwrap_or(s);
            let p = Path::new(s);
            if p.is_absolute() || s.contains(":/") || s.contains(":\\") {
                norm(s)
            } else {
                norm(&base.join(&source_root).join(s).to_string_lossy())
            }
        })
        .collect();
    (Some(map), sources)
}

fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("aGVsbG8gd29ybGQ="), b"hello world");
    }

    #[test]
    fn maps_original_lines() {
        // a.ts line 0 → generated line 2
        let map = r#"{"version":3,"sources":["a.ts"],"names":[],"mappings":";;AAAA"}"#;
        let m = SourceMap::from_slice(map.as_bytes()).unwrap();
        assert_eq!(generated_location(&m, 0, 0), Some((2, 0)));
    }
}
