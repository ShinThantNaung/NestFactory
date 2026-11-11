//! IntelliSense from the project's own TypeScript language server (`tsserver`), the same one
//! VS Code runs: completions with auto-imports from every module and package, quick fixes such
//! as "Add import", organize imports, rename across files, references, live diagnostics.
//!
//! The UI speaks the tsserver protocol (absolute paths, 1-based line and offset); this module
//! keeps one `tsserver` per project, frames its messages and matches responses to requests.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::{json, Value};
use ts_rs::TS;

use crate::process::command;

/// The message the UI looks for to restart the server and reopen its buffers.
pub const NOT_RUNNING: &str = "The TypeScript server isn't running";

/// The requests the UI may send. Everything else is refused.
const ALLOWED: &[&str] = &[
    "updateOpen",
    "quickinfo",
    "completionInfo",
    "completionEntryDetails",
    "signatureHelp",
    "definitionAndBoundSpan",
    "typeDefinition",
    "implementation",
    "references",
    "fileReferences",
    "rename",
    "documentHighlights",
    "navtree",
    "navto",
    "getCodeFixes",
    "getCombinedCodeFix",
    "getApplicableRefactors",
    "getEditsForRefactor",
    "organizeImports",
    "getEditsForFileRename",
    "syntacticDiagnosticsSync",
    "semanticDiagnosticsSync",
    "suggestionDiagnosticsSync",
    "provideInlayHints",
    "formatonkey",
    "docCommentTemplate",
    "reloadProjects",
];

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TsServerInfo {
    /// The TypeScript version, from its package.json.
    pub version: String,
    /// The tsserver.js that runs.
    pub path: String,
}

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>;

struct Session {
    root: PathBuf,
    info: TsServerInfo,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    seq: AtomicU64,
    alive: Arc<AtomicBool>,
}

/// The project's TypeScript: `node_modules/typescript` in the project or a parent folder
/// (workspaces hoist it).
fn find_typescript(root: &Path) -> Option<(PathBuf, String)> {
    for dir in root.ancestors().take(6) {
        let ts = dir.join("node_modules").join("typescript");
        let server = ts.join("lib").join("tsserver.js");
        if server.is_file() {
            let version = std::fs::read_to_string(ts.join("package.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v.get("version").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| "unknown".into());
            return Some((server, version));
        }
    }
    None
}

/// Single or double quotes for added imports, from the project's Prettier config, so a file
/// with no imports yet still gets `'@nestjs/common'` in a Nest project. "auto" (TypeScript
/// follows the file) without a config.
fn quote_preference(root: &Path) -> &'static str {
    let mut configs: Vec<String> = [".prettierrc", ".prettierrc.json", ".prettierrc.json5", ".prettierrc.yaml", ".prettierrc.yml"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(root.join(f)).ok())
        .collect();
    if let Some(p) = std::fs::read_to_string(root.join("package.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.get("prettier").cloned()) {
        configs.push(p.to_string());
    }
    for c in &configs {
        let compact: String = c.chars().filter(|ch| !ch.is_whitespace() && !matches!(*ch, '"' | '\'')).collect();
        if compact.contains("singleQuote:true") {
            return "single";
        }
        if compact.contains("singleQuote:false") {
            return "double";
        }
    }
    // A Prettier config without singleQuote means Prettier's default, double quotes.
    if configs.is_empty() {
        "auto"
    } else {
        "double"
    }
}

/// Reads `Content-Length`-framed messages and hands each response to its waiting request.
fn read_messages(stdout: ChildStdout, pending: Pending, alive: Arc<AtomicBool>) {
    let mut r = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Some(len) = line.trim().strip_prefix("Content-Length:").and_then(|n| n.trim().parse::<usize>().ok()) else {
            continue;
        };
        // The blank line after the header.
        line.clear();
        if r.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let mut body = vec![0u8; len];
        if r.read_exact(&mut body).is_err() {
            break;
        }
        let Ok(msg) = serde_json::from_slice::<Value>(&body) else { continue };
        if msg.get("type").and_then(Value::as_str) != Some("response") {
            continue; // Events: we ask for diagnostics synchronously and need none of them.
        }
        let Some(seq) = msg.get("request_seq").and_then(Value::as_u64) else { continue };
        if let Some(tx) = pending.lock().unwrap().remove(&seq) {
            let _ = tx.send(msg);
        }
    }
    alive.store(false, Ordering::SeqCst);
    // Dropping the senders wakes every waiting request with an error.
    pending.lock().unwrap().clear();
}

impl Session {
    fn start(root: &Path) -> Result<Self> {
        let (server, version) = find_typescript(root).ok_or_else(|| anyhow!("TypeScript isn't installed in this project (node_modules/typescript). Install the project's packages for full IntelliSense."))?;
        let args = vec![
            // Large projects with many packages need more than Node's default heap.
            "--max-old-space-size=3072".to_string(),
            server.to_string_lossy().to_string(),
            "--useInferredProjectPerProjectRoot".into(),
            "--disableAutomaticTypingAcquisition".into(),
            "--suppressDiagnosticEvents".into(),
            "--noGetErrOnBackgroundUpdate".into(),
            "--locale".into(),
            "en".into(),
        ];
        let mut cmd = command("node", &args, root, &[])?;
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| anyhow!("starting Node for the TypeScript server: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let pending: Pending = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        {
            let (pending, alive) = (pending.clone(), alive.clone());
            std::thread::spawn(move || read_messages(stdout, pending, alive));
        }
        let s = Session {
            root: root.to_path_buf(),
            info: TsServerInfo { version, path: server.to_string_lossy().to_string() },
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            seq: AtomicU64::new(0),
            alive,
        };
        s.configure()?;
        Ok(s)
    }

    /// The preferences VS Code uses, so completions, fixes and imports behave the same. Imports
    /// are relative within the project: Nest projects set `baseUrl: "./"`, and the `src/...`
    /// paths "shortest" would pick don't resolve in the compiled `dist`.
    fn configure(&self) -> Result<()> {
        let preferences = json!({
            "quotePreference": quote_preference(&self.root),
            "importModuleSpecifierPreference": "project-relative",
            "importModuleSpecifierEnding": "auto",
            "includePackageJsonAutoImports": "auto",
            "includeCompletionsForModuleExports": true,
            "includeCompletionsForImportStatements": true,
            "includeCompletionsWithSnippetText": true,
            "includeCompletionsWithInsertText": true,
            "includeAutomaticOptionalChainCompletions": true,
            "includeCompletionsWithClassMemberSnippets": true,
            "includeCompletionsWithObjectLiteralMethodSnippets": true,
            "useLabelDetailsInCompletionEntries": true,
            "allowIncompleteCompletions": true,
            "allowTextChangesInNewFiles": true,
            "allowRenameOfImportPath": false,
            "providePrefixAndSuffixTextForRename": true,
            "provideRefactorNotApplicableReason": true,
            "generateReturnInDocTemplate": true,
            "organizeImportsIgnoreCase": "auto",
            "excludeLibrarySymbolsInNavTo": true,
            "displayPartsForJSDoc": false,
        });
        let format = json!({
            "tabSize": 2,
            "indentSize": 2,
            "convertTabsToSpaces": true,
            "newLineCharacter": "\n",
            "semicolons": "ignore",
            "insertSpaceAfterOpeningAndBeforeClosingNonemptyBraces": true,
            "insertSpaceAfterCommaDelimiter": true,
            "insertSpaceBeforeAndAfterBinaryOperators": true,
            "insertSpaceAfterKeywordsInControlFlowStatements": true,
        });
        self.request("configure", json!({ "hostInfo": "NestFactory", "preferences": preferences, "formatOptions": format }), Duration::from_secs(20))?;
        // Files outside any tsconfig still read as Nest code.
        self.send(
            "compilerOptionsForInferredProjects",
            json!({ "options": { "target": "ES2021", "module": "CommonJS", "moduleResolution": "Node", "experimentalDecorators": true, "emitDecoratorMetadata": true, "allowJs": true, "allowNonTsExtensions": true } }),
            None,
        )?;
        Ok(())
    }

    fn send(&self, command: &str, arguments: Value, seq: Option<u64>) -> Result<()> {
        let seq = seq.unwrap_or_else(|| self.seq.fetch_add(1, Ordering::SeqCst) + 1);
        let msg = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments }).to_string();
        let mut stdin = self.stdin.lock().unwrap();
        if writeln!(stdin, "{msg}").and_then(|_| stdin.flush()).is_err() {
            self.alive.store(false, Ordering::SeqCst);
            bail!(NOT_RUNNING);
        }
        Ok(())
    }

    fn request(&self, command: &str, arguments: Value, timeout: Duration) -> Result<Value> {
        if !self.alive.load(Ordering::SeqCst) {
            bail!(NOT_RUNNING);
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(seq, tx);
        if let Err(e) = self.send(command, arguments, Some(seq)) {
            self.pending.lock().unwrap().remove(&seq);
            return Err(e);
        }
        match rx.recv_timeout(timeout) {
            Ok(msg) => {
                if msg.get("success").and_then(Value::as_bool) == Some(true) {
                    Ok(msg.get("body").cloned().unwrap_or(Value::Null))
                } else {
                    bail!("{}", msg.get("message").and_then(Value::as_str).unwrap_or("TypeScript couldn't answer"))
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&seq);
                bail!("TypeScript didn't answer '{command}' within {} seconds", timeout.as_secs())
            }
            Err(RecvTimeoutError::Disconnected) => bail!(NOT_RUNNING),
        }
    }

    fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[derive(Default)]
pub struct TsServer {
    session: Mutex<Option<Arc<Session>>>,
}

impl TsServer {
    /// Starts a fresh tsserver for the project, replacing any running one: the UI opens all its
    /// buffers again after this, so the server's open files always match the UI's.
    pub fn start(&self, root: &Path) -> Result<TsServerInfo> {
        let mut slot = self.session.lock().unwrap();
        if let Some(s) = slot.take() {
            s.stop();
        }
        let s = Arc::new(Session::start(root)?);
        let info = s.info.clone();
        *slot = Some(s);
        Ok(info)
    }

    /// Sends one request. `sync` (the arguments of an `updateOpen`) goes first, without waiting
    /// for its answer: tsserver handles requests in order, so the request sees the edits.
    pub fn request(&self, root: &Path, command: &str, args: Value, sync: Option<Value>, timeout: Duration) -> Result<Value> {
        if !ALLOWED.contains(&command) {
            bail!("'{command}' isn't a TypeScript request NestFactory sends");
        }
        let s = {
            let slot = self.session.lock().unwrap();
            match slot.as_ref() {
                Some(s) if s.root == root && s.alive.load(Ordering::SeqCst) => s.clone(),
                _ => bail!(NOT_RUNNING),
            }
        };
        if let Some(sync) = sync.filter(|v| !v.is_null()) {
            s.send("updateOpen", sync, None)?;
        }
        s.request(command, args, timeout)
    }

    pub fn stop(&self) {
        if let Some(s) = self.session.lock().unwrap().take() {
            s.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the workspace's own TypeScript when it is installed (npm install).
    #[test]
    fn completes_and_auto_imports() {
        let ws = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if find_typescript(&ws).is_none() || crate::process::resolve_program("node").is_none() {
            eprintln!("skipped: no node or node_modules/typescript");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Point the project at the workspace's TypeScript.
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        let ts_src = ws.join("node_modules").join("typescript");
        copy_dir(&ts_src, &root.join("node_modules").join("typescript"));
        std::fs::write(root.join("tsconfig.json"), r#"{ "compilerOptions": { "experimentalDecorators": true, "module": "commonjs", "target": "es2021", "baseUrl": "./" } }"#).unwrap();
        std::fs::write(root.join("package.json"), r#"{ "name": "t" }"#).unwrap();
        std::fs::write(root.join(".prettierrc"), "{
  \"singleQuote\": true
}
").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/users.service.ts"), "export class UsersService {\n  findAll() { return []; }\n}\n").unwrap();
        let app = "export const providers = [UsersSer];\n";
        std::fs::write(root.join("src/app.module.ts"), app).unwrap();

        let server = TsServer::default();
        let info = server.start(root).unwrap();
        assert!(!info.version.is_empty());
        let file = root.join("src/app.module.ts").to_string_lossy().replace('\\', "/");
        let open = json!({ "openFiles": [{ "file": file, "fileContent": app, "projectRootPath": root.to_string_lossy().replace('\\', "/") }] });
        let t = Duration::from_secs(60);
        let body = server.request(root, "completionInfo", json!({ "file": file, "line": 1, "offset": 35, "includeExternalModuleExports": true }), Some(open), t).unwrap();
        let entry = body["entries"].as_array().unwrap().iter().find(|e| e["name"] == "UsersService").cloned().expect("UsersService offered");
        assert_eq!(entry["hasAction"], true, "offered as an auto-import: {entry}");
        let details = server
            .request(root, "completionEntryDetails", json!({ "file": file, "line": 1, "offset": 35, "entryNames": [{ "name": "UsersService", "source": entry["source"], "data": entry["data"] }] }), None, t)
            .unwrap();
        let edit = details[0]["codeActions"][0]["changes"][0]["textChanges"][0]["newText"].as_str().unwrap().to_string();
        assert!(edit.contains("import { UsersService } from './users.service'"), "relative, single-quoted: {edit}");

        let diags = server.request(root, "semanticDiagnosticsSync", json!({ "file": file }), None, t).unwrap();
        assert!(diags.as_array().unwrap().iter().any(|d| d["code"] == 2552 || d["code"] == 2304), "{diags}");
        assert!(server.request(root, "exit", json!({}), None, t).is_err(), "only allowed commands");
        server.stop();
        assert!(server.request(root, "quickinfo", json!({}), None, t).unwrap_err().to_string().contains(NOT_RUNNING));
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                copy_dir(&p, &to.join(e.file_name()));
            } else {
                std::fs::copy(&p, to.join(e.file_name())).unwrap();
            }
        }
    }
}
