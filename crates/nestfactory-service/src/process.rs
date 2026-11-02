//! Child processes: dev server, tests, installs, the CLI. Output is streamed line by line as
//! `process-output` events. On Windows, `.cmd` shims are resolved from PATH, no console
//! window is opened, and stopping a process kills its whole tree.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::EventSink;

pub struct ProcessManager {
    sink: Arc<dyn EventSink>,
    procs: Mutex<HashMap<String, u32>>,
}

/// Finds an executable on PATH, trying Windows extensions (`npm` → `npm.cmd`).
pub fn resolve_program(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        let mut v: Vec<String> = pathext.split(';').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect();
        v.insert(0, String::new());
        v
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() && (!cfg!(windows) || !ext.is_empty() || name.contains('.')) {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn command(program: &str, args: &[String], cwd: &Path, env: &[(String, String)]) -> Result<Command> {
    let exe = resolve_program(program).ok_or_else(|| anyhow!("{program} was not found on PATH"))?;
    let mut cmd = Command::new(exe);
    cmd.args(args).current_dir(cwd);
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(cmd)
}

fn pump<R: Read + Send + 'static>(reader: R, id: String, stream: &'static str, sink: Arc<dyn EventSink>, collect: Option<Arc<Mutex<String>>>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut r = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf).trim_end_matches(['\n', '\r']).to_string();
                    if let Some(c) = &collect {
                        let mut c = c.lock().unwrap();
                        c.push_str(&line);
                        c.push('\n');
                    }
                    sink.emit("process-output", json!({ "id": id, "stream": stream, "text": line }));
                }
            }
        }
    })
}

impl ProcessManager {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self { sink, procs: Mutex::new(HashMap::new()) }
    }

    pub fn running(&self) -> Vec<String> {
        self.procs.lock().unwrap().keys().cloned().collect()
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.procs.lock().unwrap().contains_key(id)
    }

    fn start(&self, id: &str, mut cmd: Command, collect: Option<Arc<Mutex<String>>>) -> Result<(Child, Vec<std::thread::JoinHandle<()>>)> {
        self.kill(id);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().context("starting process")?;
        let pid = child.id();
        self.procs.lock().unwrap().insert(id.to_string(), pid);
        self.sink.emit("process-start", json!({ "id": id, "pid": pid }));
        let mut handles = vec![];
        if let Some(out) = child.stdout.take() {
            handles.push(pump(out, id.to_string(), "stdout", self.sink.clone(), collect.clone()));
        }
        if let Some(err) = child.stderr.take() {
            handles.push(pump(err, id.to_string(), "stderr", self.sink.clone(), collect));
        }
        Ok((child, handles))
    }

    /// Starts a long-running process (dev server, debugger) and returns immediately.
    pub fn spawn(self: &Arc<Self>, id: &str, cmd: Command) -> Result<u32> {
        let (mut child, handles) = self.start(id, cmd, None)?;
        let pid = child.id();
        let me = self.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            for h in handles {
                let _ = h.join();
            }
            let mut procs = me.procs.lock().unwrap();
            if procs.get(&id) == Some(&pid) {
                procs.remove(&id);
            }
            drop(procs);
            me.sink.emit("process-exit", json!({ "id": id, "code": code }));
        });
        Ok(pid)
    }

    /// Runs a process to completion, streaming output, and returns the exit code and output.
    pub fn run(&self, id: &str, cmd: Command) -> Result<(i32, String)> {
        let collected = Arc::new(Mutex::new(String::new()));
        let (mut child, handles) = self.start(id, cmd, Some(collected.clone()))?;
        let pid = child.id();
        let status = child.wait()?;
        for h in handles {
            let _ = h.join();
        }
        {
            let mut procs = self.procs.lock().unwrap();
            if procs.get(id) == Some(&pid) {
                procs.remove(id);
            }
        }
        let code = status.code().unwrap_or(-1);
        self.sink.emit("process-exit", json!({ "id": id, "code": code }));
        let out = collected.lock().unwrap().clone();
        Ok((code, out))
    }

    pub fn kill(&self, id: &str) -> bool {
        let pid = self.procs.lock().unwrap().remove(id);
        let Some(pid) = pid else { return false };
        kill_tree(pid);
        true
    }

    pub fn kill_all(&self) {
        let pids: Vec<u32> = self.procs.lock().unwrap().drain().map(|(_, p)| p).collect();
        for p in pids {
            kill_tree(p);
        }
    }
}

/// Ends a process and everything it started.
pub(crate) fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-TERM", &format!("-{pid}")]).status();
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
}
