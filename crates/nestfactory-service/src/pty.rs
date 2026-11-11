//! Interactive terminals: real shells (PowerShell, Command Prompt, Git Bash, WSL, Node.js…) on a
//! pseudo-console (ConPTY on Windows), as VS Code runs them. Output goes to xterm.js in the UI as
//! `pty-output` events; keystrokes come back through `write`. `pty-exit` follows the shell's end.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

#[cfg(windows)]
use crate::process::command;
use crate::process::{kill_tree, resolve_program};
use crate::EventSink;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalProfile {
    /// Stable id kept in settings: `pwsh`, `powershell`, `cmd`, `gitbash`, `wsl:<distro>`, `node`, `bash`…
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    #[ts(skip)]
    #[serde(skip)]
    pub env: Vec<(String, String)>,
}

fn profile(id: &str, name: &str, program: PathBuf, args: &[&str]) -> TerminalProfile {
    TerminalProfile { id: id.into(), name: name.into(), program: program.to_string_lossy().to_string(), args: args.iter().map(|s| s.to_string()).collect(), env: vec![] }
}

/// The shells on this machine, the default (PowerShell on Windows) first. Detected once.
pub fn profiles() -> &'static [TerminalProfile] {
    static PROFILES: OnceLock<Vec<TerminalProfile>> = OnceLock::new();
    PROFILES.get_or_init(detect)
}

#[cfg(windows)]
fn detect() -> Vec<TerminalProfile> {
    let mut out = vec![];
    let pwsh = resolve_program("pwsh");
    let windows_ps = resolve_program("powershell");
    match (&pwsh, &windows_ps) {
        (Some(p), Some(w)) => {
            out.push(profile("pwsh", "PowerShell", p.clone(), &["-NoLogo"]));
            out.push(profile("powershell", "Windows PowerShell", w.clone(), &["-NoLogo"]));
        }
        (Some(p), None) => out.push(profile("pwsh", "PowerShell", p.clone(), &["-NoLogo"])),
        (None, Some(w)) => out.push(profile("powershell", "PowerShell", w.clone(), &["-NoLogo"])),
        (None, None) => {}
    }
    let comspec = std::env::var_os("ComSpec").map(PathBuf::from).filter(|p| p.is_file());
    if let Some(cmd) = comspec.or_else(|| resolve_program("cmd")) {
        out.push(profile("cmd", "Command Prompt", cmd, &[]));
    }
    if let Some(bash) = git_bash() {
        let mut p = profile("gitbash", "Git Bash", bash, &["--login", "-i"]);
        // Keeps the login shell in the project folder instead of going to $HOME.
        p.env.push(("CHERE_INVOKING".into(), "1".into()));
        out.push(p);
    }
    for distro in wsl_distros() {
        let wsl = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\wsl.exe");
        out.push(profile(&format!("wsl:{distro}"), &format!("{distro} (WSL)"), wsl, &["-d", &distro]));
    }
    if let Some(node) = resolve_program("node") {
        out.push(profile("node", "Node.js", node, &[]));
    }
    out
}

/// Git for Windows' bash.exe (not System32\bash.exe, which is WSL's).
#[cfg(windows)]
fn git_bash() -> Option<PathBuf> {
    let mut candidates = vec![];
    if let Some(git) = resolve_program("git") {
        // …\Git\cmd\git.exe, …\Git\bin\git.exe or …\Git\mingw64\bin\git.exe
        for up in git.ancestors().skip(1).take(3) {
            candidates.push(up.join(r"bin\bash.exe"));
        }
    }
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(dir) = std::env::var(var) {
            candidates.push(Path::new(&dir).join(r"Git\bin\bash.exe"));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(Path::new(&local).join(r"Programs\Git\bin\bash.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Installed WSL distributions, without Docker Desktop's internal ones.
#[cfg(windows)]
fn wsl_distros() -> Vec<String> {
    let wsl = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\wsl.exe");
    if !wsl.is_file() {
        return vec![];
    }
    let Ok(mut cmd) = command(&wsl.to_string_lossy(), &["-l".into(), "-q".into()], &std::env::temp_dir(), &[]) else { return vec![] };
    let Ok(out) = output_within(&mut cmd, Duration::from_secs(4)) else { return vec![] };
    // wsl.exe writes UTF-16LE.
    let units: Vec<u16> = out.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop"))
        .collect()
}

#[cfg(not(windows))]
fn detect() -> Vec<TerminalProfile> {
    let mut out = vec![];
    let login = std::env::var("SHELL").ok().map(PathBuf::from).filter(|p| p.is_file());
    let mut seen = vec![];
    for sh in login.into_iter().chain(["zsh", "bash", "fish", "sh"].iter().filter_map(|s| resolve_program(s))) {
        let name = sh.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        out.push(profile(&name, &name, sh, &["-l"]));
    }
    if let Some(node) = resolve_program("node") {
        out.push(profile("node", "Node.js", node, &[]));
    }
    out
}

/// Runs a command for its stdout, giving up (and killing it) after `limit`.
#[cfg(windows)]
fn output_within(cmd: &mut std::process::Command, limit: Duration) -> Result<Vec<u8>> {
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![];
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(limit) {
        Ok(buf) => {
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            if ok { Ok(buf) } else { Err(anyhow!("exited with an error")) }
        }
        Err(_) => {
            let _ = child.kill();
            Err(anyhow!("timed out"))
        }
    }
}

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
}

pub struct Terminals {
    sink: Arc<dyn EventSink>,
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
}

/// Splits off the longest valid UTF-8 prefix, keeping a sequence cut by the read for next time.
fn take_utf8(pending: &mut Vec<u8>) -> String {
    match std::str::from_utf8(pending) {
        Ok(s) => {
            let s = s.to_string();
            pending.clear();
            s
        }
        Err(e) if e.error_len().is_none() => {
            let valid = e.valid_up_to();
            let s = String::from_utf8_lossy(&pending[..valid]).into_owned();
            pending.drain(..valid);
            s
        }
        Err(_) => {
            let s = String::from_utf8_lossy(pending).into_owned();
            pending.clear();
            s
        }
    }
}

impl Terminals {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self { sink, sessions: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Starts `profile` (or the default) in `cwd` as terminal `id`, sized `cols` × `rows`.
    pub fn spawn(&self, id: &str, profile_id: Option<&str>, cwd: &Path, cols: u16, rows: u16) -> Result<TerminalProfile> {
        let all = profiles();
        let p = profile_id.and_then(|want| all.iter().find(|p| p.id == want)).or_else(|| all.first()).ok_or_else(|| anyhow!("No shell was found on this machine"))?.clone();
        let size = PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).context("opening a pseudo-console")?;
        let mut cmd = CommandBuilder::new(&p.program);
        cmd.args(&p.args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "NestFactory");
        for (k, v) in &p.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("starting {}", p.name))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let session = Arc::new(Session { pid: child.process_id(), killer: Mutex::new(child.clone_killer()), writer: Mutex::new(writer), master: Mutex::new(pair.master) });
        self.sessions.lock().unwrap().insert(id.to_string(), session);

        let sink = self.sink.clone();
        let tid = id.to_string();
        let pump = std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            let mut pending = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let data = take_utf8(&mut pending);
                        if !data.is_empty() {
                            sink.emit("pty-output", json!({ "id": tid, "data": data }));
                        }
                    }
                }
            }
        });

        let sink = self.sink.clone();
        let sessions = self.sessions.clone();
        let tid = id.to_string();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code()).ok();
            // Dropping the session closes the pseudo-console, which ends the reader; joining it
            // first sends the shell's last words before the exit.
            sessions.lock().unwrap().remove(&tid);
            let _ = pump.join();
            sink.emit("pty-exit", json!({ "id": tid, "code": code }));
        });
        Ok(p)
    }

    fn get(&self, id: &str) -> Result<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned().ok_or_else(|| anyhow!("That terminal has closed"))
    }

    pub fn write(&self, id: &str, data: &str) -> Result<()> {
        let s = self.get(id)?;
        let mut w = s.writer.lock().unwrap();
        w.write_all(data.as_bytes())?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let s = self.get(id)?;
        let size = PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 };
        s.master.lock().unwrap().resize(size)?;
        Ok(())
    }

    /// Ends the shell and everything it started (a dev server, a watcher…).
    pub fn kill(&self, id: &str) {
        let Ok(s) = self.get(id) else { return };
        if let Some(pid) = s.pid {
            kill_tree(pid);
        }
        let _ = s.killer.lock().unwrap().kill();
    }

    pub fn kill_all(&self) {
        let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.kill(&id);
        }
    }
}
