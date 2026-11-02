//! Formatting with the project's own Prettier, as the VS Code extension does: its version, its
//! `.prettierrc` and `.prettierignore`. Loading Prettier takes most of a format call, so one
//! small Node process stays up per project and answers a JSON request per line.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use nestfactory_core::testing::strip_ansi;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use crate::process::command;

/// Requests can overlap in Node, so each reply carries its request's id. Prettier is required
/// again while missing, so installing it later needs no restart.
const DAEMON: &str = r#"
const path = require('path');
const { createRequire } = require('module');
const root = process.argv[1];
let prettier = null;
function load() {
  if (prettier) return prettier;
  try { prettier = createRequire(path.join(root, 'package.json'))('prettier'); } catch { prettier = null; }
  return prettier;
}
load();
require('readline').createInterface({ input: process.stdin }).on('line', async (line) => {
  let req;
  try { req = JSON.parse(line); } catch { return; }
  const reply = (r) => process.stdout.write(JSON.stringify({ id: req.id, ...r }) + '\n');
  const p = load();
  if (!p) return reply({ ok: true, text: null, formatter: null });
  try {
    const file = path.join(root, req.path);
    if (p.clearConfigCache) await p.clearConfigCache();
    const info = await p.getFileInfo(file, { ignorePath: path.join(root, '.prettierignore'), resolveConfig: true });
    if (info.ignored || !info.inferredParser) return reply({ ok: true, text: null, formatter: 'prettier', version: p.version });
    const options = (await p.resolveConfig(file, { editorconfig: true })) || {};
    const text = await p.format(req.text, { ...options, filepath: file });
    reply({ ok: true, text, formatter: 'prettier', version: p.version });
  } catch (e) {
    reply({ ok: false, error: String((e && e.message) || e), formatter: 'prettier', version: p.version });
  }
});
"#;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FormatResult {
    /// The formatted text; null when the file wasn't formatted (no Prettier in the project, the
    /// file is in .prettierignore, or Prettier has no parser for it).
    pub text: Option<String>,
    /// "prettier", or null when the project has no Prettier.
    pub formatter: Option<String>,
    pub version: Option<String>,
    /// Prettier's message when it couldn't format, usually a syntax error.
    pub error: Option<String>,
}

struct Daemon {
    root: PathBuf,
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<String>,
    next: u64,
}

impl Daemon {
    fn start(root: &Path) -> Result<Self> {
        let mut cmd = command("node", &["-e".to_string(), DAEMON.to_string(), root.to_string_lossy().to_string()], root, &[])?;
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| anyhow!("starting Node for Prettier: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Daemon { root: root.to_path_buf(), child, stdin, replies, next: 0 })
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct Formatter {
    daemon: Mutex<Option<Daemon>>,
}

const TIMEOUT: Duration = Duration::from_secs(20);

impl Formatter {
    pub fn format(&self, root: &Path, rel: &str, text: &str) -> Result<FormatResult> {
        let mut slot = self.daemon.lock().unwrap();
        if slot.as_mut().is_some_and(|d| d.root != root || !matches!(d.child.try_wait(), Ok(None))) {
            slot.take().unwrap().stop();
        }
        if slot.is_none() {
            *slot = Some(Daemon::start(root)?);
        }
        let d = slot.as_mut().unwrap();
        d.next += 1;
        let id = d.next;
        let line = json!({ "id": id, "path": rel, "text": text }).to_string();
        if writeln!(d.stdin, "{line}").and_then(|_| d.stdin.flush()).is_err() {
            slot.take().unwrap().stop();
            bail!("The Prettier process stopped; save again to restart it");
        }
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let Ok(reply) = d.replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) else {
                slot.take().unwrap().stop();
                bail!("Prettier didn't answer within {} seconds", TIMEOUT.as_secs());
            };
            let v: Value = serde_json::from_str(&reply).unwrap_or(Value::Null);
            // A late reply to a request that timed out earlier; skip it.
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
            // Prettier colours its code frames when the app was started with FORCE_COLOR.
            let error = s("error").map(|e| strip_ansi(&e));
            return Ok(FormatResult { text: s("text"), formatter: s("formatter"), version: s("version"), error });
        }
    }

    pub fn stop(&self) {
        if let Some(d) = self.daemon.lock().unwrap().take() {
            d.stop();
        }
    }
}
