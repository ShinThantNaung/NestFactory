//! NestFactory backend. One [`Backend`] serves the desktop shell (Tauri) and the dev bridge
//! (a local HTTP server for running the UI in a browser). Both call [`Backend::dispatch`]
//! with a command name and JSON arguments; events go out through an [`EventSink`].

pub mod debug;
pub mod http;
pub mod packages;
pub mod process;
pub mod store;

use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use nestfactory_core::changeset::{self, Changeset};
use nestfactory_core::db::{self, DbConfig, DbPlan};
use nestfactory_core::entity::{self, EntityOptions};
use nestfactory_core::env::{self as envm, EnvReport};
use nestfactory_core::fixes;
use nestfactory_core::generate::{self, GenerateOptions};
use nestfactory_core::model::{FixAction, PackageManager, ProjectModel};
use nestfactory_core::pm::PackageAction;
use nestfactory_core::project::read_meta;
use nestfactory_core::testing::{self, Problem, TestRun};
use nestfactory_core::wizard::{self, PlannedFile, WizardOptions, WizardStep};
use nestfactory_core::wiring;
use nestfactory_core::{pm, Scanner};
use notify::Watcher;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use crate::http::{ApiRequest, Endpoint, HistoryEntry};
use crate::process::{command, resolve_program, shell_command, ProcessManager};
use crate::store::{DbProfile, Settings, Store};

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value);
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub ignored: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DbTestResult {
    pub ok: bool,
    pub latency_ms: f64,
    pub version: Option<String>,
    pub error: Option<String>,
    /// True when only TCP reachability was checked (driver not installed yet).
    pub tcp_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DockerStatus {
    pub available: bool,
    pub exists: bool,
    pub running: bool,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Toolchain {
    pub node: Option<String>,
    pub nest: Option<String>,
    pub docker: Option<String>,
    pub git: Option<String>,
    pub package_managers: Vec<PackageManager>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WizardPlan {
    pub steps: Vec<WizardStep>,
    pub files: Vec<PlannedFile>,
    pub error: Option<String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryItem {
    pub id: String,
    pub label: String,
    pub files: u32,
}

struct State {
    root: Option<PathBuf>,
    scanner: Scanner,
    model: Option<Arc<ProjectModel>>,
    pending: HashMap<String, Changeset>,
    undo: Vec<Changeset>,
    redo: Vec<Changeset>,
    watcher: Option<notify::RecommendedWatcher>,
    generation: u64,
}

struct Inner {
    sink: Arc<dyn EventSink>,
    store: Store,
    procs: Arc<ProcessManager>,
    state: Mutex<State>,
    debug: Mutex<Option<debug::Session>>,
    /// A package manager command from the package view is running; one at a time.
    packages_busy: AtomicBool,
}

#[derive(Clone)]
pub struct Backend {
    inner: Arc<Inner>,
}

fn arg<T: serde::de::DeserializeOwned>(a: &Value, name: &str) -> Result<T> {
    serde_json::from_value(a.get(name).cloned().unwrap_or(Value::Null)).with_context(|| format!("invalid argument '{name}'"))
}

fn opt_arg<T: serde::de::DeserializeOwned>(a: &Value, name: &str) -> Option<T> {
    a.get(name).cloned().filter(|v| !v.is_null()).and_then(|v| serde_json::from_value(v).ok())
}

fn val<T: Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

const IGNORED_DIRS: &[&str] = &["node_modules", "dist", "coverage", ".git", "build", "target"];

impl Backend {
    pub fn new(data_dir: PathBuf, sink: Arc<dyn EventSink>) -> Self {
        let procs = Arc::new(ProcessManager::new(sink.clone()));
        Backend {
            inner: Arc::new(Inner {
                sink,
                store: Store::new(data_dir),
                procs,
                state: Mutex::new(State {
                    root: None,
                    scanner: Scanner::new(),
                    model: None,
                    pending: HashMap::new(),
                    undo: vec![],
                    redo: vec![],
                    watcher: None,
                    generation: 0,
                }),
                debug: Mutex::new(None),
                packages_busy: AtomicBool::new(false),
            }),
        }
    }

    pub fn emit(&self, event: &str, payload: Value) {
        self.inner.sink.emit(event, payload);
    }

    pub fn shutdown(&self) {
        if let Some(s) = self.inner.debug.lock().unwrap().take() {
            s.stop();
        }
        self.inner.procs.kill_all();
    }

    /// Runs a command. Errors are returned as display strings for the UI.
    pub fn dispatch(&self, cmd: &str, args: Value) -> std::result::Result<Value, String> {
        self.handle(cmd, &args).map_err(|e| format!("{e:#}"))
    }

    fn root(&self) -> Result<PathBuf> {
        self.inner.state.lock().unwrap().root.clone().ok_or_else(|| anyhow!("No project is open"))
    }

    fn model(&self) -> Result<Arc<ProjectModel>> {
        let st = self.inner.state.lock().unwrap();
        st.model.clone().ok_or_else(|| anyhow!("No project is open"))
    }

    fn package_manager(&self) -> PackageManager {
        self.model().map(|m| m.package_manager).unwrap_or(PackageManager::Npm)
    }

    /// Resolves a project-relative path, refusing anything outside the project.
    fn path_in_root(&self, rel: &str) -> Result<PathBuf> {
        let root = self.root()?;
        let mut out = root.clone();
        for c in Path::new(rel).components() {
            match c {
                Component::Normal(p) => out.push(p),
                Component::CurDir => {}
                _ => bail!("Path must stay inside the project: {rel}"),
            }
        }
        Ok(out)
    }

    fn handle(&self, cmd: &str, a: &Value) -> Result<Value> {
        match cmd {
            // ---- app --------------------------------------------------------------------------
            "getSettings" => val(self.inner.store.settings()),
            "saveSettings" => {
                let s: Settings = arg(a, "settings")?;
                self.inner.store.save_settings(&s)?;
                val(s)
            }
            "toolchain" => val(toolchain()),
            "recentProjects" => val(self.inner.store.recent_projects()),
            "removeRecent" => {
                let path: Option<String> = opt_arg(a, "path");
                self.inner.store.remove_recent(path.as_deref())?;
                val(self.inner.store.recent_projects())
            }

            // ---- project ----------------------------------------------------------------------
            "openProject" => {
                let path: String = arg(a, "path")?;
                val(&*self.open_project(Path::new(&path))?)
            }
            "closeProject" => {
                let mut st = self.inner.state.lock().unwrap();
                st.root = None;
                st.model = None;
                st.watcher = None;
                st.generation += 1;
                st.undo.clear();
                st.redo.clear();
                drop(st);
                self.inner.procs.kill_all();
                val(true)
            }
            "rescan" => val(&*self.rescan()?),
            "listDir" => {
                let rel: String = opt_arg(a, "path").unwrap_or_default();
                val(self.list_dir(&rel)?)
            }
            "listFiles" => val(self.list_files()?),
            "readFile" => {
                let rel: String = arg(a, "path")?;
                let p = self.path_in_root(&rel)?;
                let content = std::fs::read_to_string(&p).with_context(|| format!("reading {rel}"))?;
                val(json!({ "content": content }))
            }
            "writeFile" => {
                let rel: String = arg(a, "path")?;
                let content: String = arg(a, "content")?;
                let p = self.path_in_root(&rel)?;
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&p, content).with_context(|| format!("writing {rel}"))?;
                val(true)
            }
            "deleteFile" => {
                let rel: String = arg(a, "path")?;
                let p = self.path_in_root(&rel)?;
                if p.is_dir() {
                    std::fs::remove_dir_all(&p)?;
                } else {
                    std::fs::remove_file(&p)?;
                }
                val(true)
            }
            "renameFile" => {
                let from = self.path_in_root(&arg::<String>(a, "from")?)?;
                let to = self.path_in_root(&arg::<String>(a, "to")?)?;
                if to.exists() {
                    bail!("{} already exists", to.display());
                }
                std::fs::rename(from, to)?;
                val(true)
            }

            // ---- generation and fixes -------------------------------------------------------------
            "previewGenerate" => {
                let opts: GenerateOptions = arg(a, "options")?;
                let root = self.root()?;
                let cs = generate::generate(&root, &read_meta(&root), &opts)?;
                self.remember(cs)
            }
            "previewFix" => {
                let action: FixAction = arg(a, "action")?;
                let cs = fixes::preview_fix(&self.root()?, &*self.model()?, &action)?;
                self.remember(cs)
            }
            "previewWire" => {
                let provider: String = arg(a, "provider")?;
                let module: String = arg(a, "module")?;
                let model = self.model()?;
                let action = fixes::wire_action(&model, &provider, &module)?;
                let cs = fixes::preview_fix(&self.root()?, &model, &action)?;
                self.remember(cs)
            }
            "previewEntity" => {
                let opts: EntityOptions = arg(a, "options")?;
                let root = self.root()?;
                let cs = entity::generate_entity(&root, &read_meta(&root), &*self.model()?, &opts)?;
                self.remember(cs)
            }
            "previewSyncDto" => {
                let name: String = arg(a, "entity")?;
                let model = self.model()?;
                let class = model.classes.iter().find(|c| c.name == name).ok_or_else(|| anyhow!("{name} not found"))?;
                let root = self.root()?;
                let cs = entity::sync_dto(&root, &read_meta(&root), class)?;
                self.remember(cs)
            }
            "applyChangeset" => {
                let id: String = arg(a, "id")?;
                val(self.apply_pending(&id)?)
            }
            "discardChangeset" => {
                let id: String = arg(a, "id")?;
                self.inner.state.lock().unwrap().pending.remove(&id);
                val(true)
            }
            "undo" => val(self.undo()?),
            "redo" => val(self.redo()?),
            "history" => {
                let st = self.inner.state.lock().unwrap();
                val(st.undo.iter().rev().map(|c| HistoryItem { id: c.id.clone(), label: c.label.clone(), files: c.files.len() as u32 }).collect::<Vec<_>>())
            }

            // ---- database -------------------------------------------------------------------------
            "dbProfiles" => val(self.inner.store.project(&self.root()?).db_profiles),
            "dbPassword" => {
                let name: String = arg(a, "name")?;
                val(store::get_password(&self.root()?, &name))
            }
            "dbPlan" => {
                let cfg: DbConfig = arg(a, "config")?;
                let root = self.root()?;
                let mut plan = db::plan(&root, &read_meta(&root), &*self.model()?, &cfg)?;
                let id = plan.changeset.id.clone();
                self.inner.state.lock().unwrap().pending.insert(id, plan.changeset.clone());
                plan.changeset.id = plan.changeset.id.clone();
                val(plan)
            }
            "dbApply" => {
                let cfg: DbConfig = arg(a, "config")?;
                let id: Option<String> = opt_arg(a, "changesetId");
                val(self.db_apply(&cfg, id)?)
            }
            "dbSaveProfile" => {
                let cfg: DbConfig = arg(a, "config")?;
                self.save_profile(&cfg, None)?;
                val(self.inner.store.project(&self.root()?).db_profiles)
            }
            "dbDeleteProfile" => {
                let name: String = arg(a, "name")?;
                let root = self.root()?;
                store::delete_password(&root, &name);
                let d = self.inner.store.update_project(&root, |d| d.db_profiles.retain(|p| p.config.name != name))?;
                val(d.db_profiles)
            }
            "dbTest" => {
                let cfg: DbConfig = arg(a, "config")?;
                val(self.db_test(&cfg))
            }
            "dockerStatus" => {
                let container: String = arg(a, "container")?;
                val(docker_status(&container))
            }
            "dockerStart" | "dockerStop" => {
                let service: String = arg(a, "service")?;
                let root = self.root()?;
                let verb = if cmd == "dockerStart" { "up" } else { "stop" };
                let mut args = vec!["compose".to_string(), verb.to_string()];
                if verb == "up" {
                    args.push("-d".into());
                }
                args.push(service);
                let (code, out) = self.inner.procs.run("docker", command("docker", &args, &root, &[])?)?;
                if code != 0 {
                    bail!("docker compose {verb} failed:\n{}", tail(&out, 12));
                }
                val(true)
            }

            // ---- run, test, typecheck, terminal ------------------------------------------------------
            "runScript" => {
                let script: String = arg(a, "script")?;
                let root = self.root()?;
                let cmdline = pm::run_script(self.package_manager(), &script);
                let c = command(&cmdline[0], &cmdline[1..], &root, &[("FORCE_COLOR".into(), "1".into())])?;
                val(self.inner.procs.spawn("run", c)?)
            }
            "stopProcess" => {
                let id: String = arg(a, "id")?;
                val(self.inner.procs.kill(&id))
            }
            "processes" => val(self.inner.procs.running()),
            "runCommand" => {
                let line: String = arg(a, "line")?;
                let root = self.root()?;
                let mut c = shell_command(&line, &root);
                c.env("FORCE_COLOR", "1");
                val(self.inner.procs.spawn("terminal", c)?)
            }
            "runTests" => {
                let e2e: bool = opt_arg(a, "e2e").unwrap_or(false);
                let file: Option<String> = opt_arg(a, "file");
                let name: Option<String> = opt_arg(a, "name");
                let coverage: bool = opt_arg(a, "coverage").unwrap_or(true);
                val(self.run_tests(e2e, file, name, coverage)?)
            }
            "typecheck" => val(self.typecheck()?),

            // ---- packages -----------------------------------------------------------------------
            "pkgList" => val(self.package_list()?),
            "pkgDetails" => {
                let name: String = arg(a, "name")?;
                let root = self.root()?;
                let wiring = wiring::steps(&root, &*self.model()?, &name);
                val(packages::details(&packages::registry_url(&root), &self.package_list()?, &name, wiring))
            }
            "pkgCommand" => {
                let action: PackageAction = arg(a, "action")?;
                let specs: Vec<String> = opt_arg(a, "packages").unwrap_or_default();
                let dev: bool = opt_arg(a, "dev").unwrap_or(false);
                let dev_specs: Vec<String> = opt_arg(a, "devPackages").unwrap_or_default();
                val(packages::commands_for(self.package_manager(), action, &specs, dev, &dev_specs).iter().map(|c| pm::display(c)).collect::<Vec<_>>())
            }
            "pkgWire" => {
                let name: String = arg(a, "name")?;
                let steps: Vec<String> = arg(a, "steps")?;
                let cs = wiring::wire(&self.root()?, &*self.model()?, &name, &steps)?;
                self.remember(cs)
            }
            "pkgUnwire" => {
                let name: String = arg(a, "name")?;
                match wiring::unwire(&self.root()?, &*self.model()?, &name) {
                    Some(cs) => self.remember(cs),
                    None => Ok(Value::Null),
                }
            }
            "pkgLatest" => {
                let root = self.root()?;
                val(packages::latest_versions(&packages::registry_url(&root), &pm::list(&root)))
            }
            "pkgSearch" => {
                let query: String = arg(a, "query")?;
                let root = self.root()?;
                val(packages::search(&packages::registry_url(&root), &query, &pm::list(&root))?)
            }
            "pkgCatalog" => {
                let list = pm::list(&self.root()?);
                val(pm::catalog(&|n| list.packages.iter().any(|p| p.name == n)))
            }
            "pkgPlan" => {
                let action: PackageAction = arg(a, "action")?;
                let specs: Vec<String> = opt_arg(a, "packages").unwrap_or_default();
                let dev: bool = opt_arg(a, "dev").unwrap_or(false);
                let root = self.root()?;
                val(packages::plan(&packages::registry_url(&root), self.package_manager(), &pm::list(&root), action, &specs, dev))
            }
            "pkgRun" => {
                let action: PackageAction = arg(a, "action")?;
                let specs: Vec<String> = opt_arg(a, "packages").unwrap_or_default();
                let dev: bool = opt_arg(a, "dev").unwrap_or(false);
                let dev_specs: Vec<String> = opt_arg(a, "devPackages").unwrap_or_default();
                val(self.run_packages(action, specs, dev, dev_specs)?)
            }

            // ---- env -------------------------------------------------------------------------------
            "envReport" => val(envm::report(&self.root()?)),
            "envSave" => {
                let file: String = arg(a, "file")?;
                let updates: Vec<(String, Option<String>)> = arg(a, "updates")?;
                if !(file == ".env" || file.starts_with(".env.")) || file.contains(['/', '\\']) {
                    bail!("Only .env files in the project root can be edited here");
                }
                let p = self.path_in_root(&file)?;
                let cur = std::fs::read_to_string(&p).unwrap_or_default();
                std::fs::write(&p, envm::update_env(&cur, &updates))?;
                val::<EnvReport>(envm::report(&self.root()?))
            }

            // ---- API explorer --------------------------------------------------------------------------
            "apiEndpoints" => val(self.endpoints()?),
            "apiSend" => {
                let req: ApiRequest = arg(a, "request")?;
                let res = http::send(&req);
                let root = self.root()?;
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
                let entry = HistoryEntry { request: req, status: res.status, time_ms: res.time_ms, at };
                self.inner.store.update_project(&root, |d| {
                    d.history.insert(0, entry);
                    d.history.truncate(50);
                })?;
                val(res)
            }
            "apiSaved" => val(self.inner.store.project(&self.root()?).saved_requests),
            "apiHistory" => val(self.inner.store.project(&self.root()?).history),
            "apiSave" => {
                let mut req: ApiRequest = arg(a, "request")?;
                if req.id.is_empty() {
                    req.id = changeset::new_id();
                }
                let d = self.inner.store.update_project(&self.root()?, |d| {
                    match d.saved_requests.iter_mut().find(|r| r.id == req.id) {
                        Some(r) => *r = req.clone(),
                        None => d.saved_requests.push(req.clone()),
                    }
                })?;
                val(d.saved_requests)
            }
            "apiDelete" => {
                let id: String = arg(a, "id")?;
                let d = self.inner.store.update_project(&self.root()?, |d| d.saved_requests.retain(|r| r.id != id))?;
                val(d.saved_requests)
            }

            // ---- wizard ---------------------------------------------------------------------------------
            "wizardPlan" => {
                let opts: WizardOptions = arg(a, "options")?;
                let steps = wizard::plan(&opts);
                let commands = steps.iter().filter_map(|s| s.command.as_ref().map(|c| pm::display(c))).collect();
                val(WizardPlan { error: wizard::validate(&opts).err().map(|e| e.to_string()), files: wizard::planned_files(&opts), steps, commands })
            }
            "wizardRun" => {
                let opts: WizardOptions = arg(a, "options")?;
                val(self.wizard_run(&opts)?)
            }

            // ---- debugger ---------------------------------------------------------------------------------
            "debugStart" => {
                let mode: String = arg(a, "mode")?;
                let file: Option<String> = opt_arg(a, "file");
                let breakpoints: Vec<debug::Breakpoint> = opt_arg(a, "breakpoints").unwrap_or_default();
                let root = self.root()?;
                if let Some(old) = self.inner.debug.lock().unwrap().take() {
                    old.stop();
                }
                let session = debug::Session::start(root, self.package_manager(), &mode, file, breakpoints, self.inner.procs.clone(), self.inner.sink.clone())?;
                *self.inner.debug.lock().unwrap() = Some(session);
                val(true)
            }
            "debugSetBreakpoints" => {
                let bps: Vec<debug::Breakpoint> = arg(a, "breakpoints")?;
                if let Some(s) = self.inner.debug.lock().unwrap().as_ref() {
                    s.set_breakpoints(bps);
                }
                val(true)
            }
            "debugCommand" => {
                let c: String = arg(a, "command")?;
                let s = self.inner.debug.lock().unwrap();
                let s = s.as_ref().ok_or_else(|| anyhow!("No debug session"))?;
                s.command(&c)?;
                val(true)
            }
            "debugProperties" => {
                let object_id: String = arg(a, "objectId")?;
                let s = self.inner.debug.lock().unwrap();
                let s = s.as_ref().ok_or_else(|| anyhow!("No debug session"))?;
                val(s.properties(&object_id)?)
            }
            "debugStop" => {
                if let Some(s) = self.inner.debug.lock().unwrap().take() {
                    s.stop();
                }
                val(true)
            }
            _ => bail!("Unknown command: {cmd}"),
        }
    }

    fn remember(&self, cs: Changeset) -> Result<Value> {
        self.inner.state.lock().unwrap().pending.insert(cs.id.clone(), cs.clone());
        val(cs)
    }

    pub fn open_project(&self, path: &Path) -> Result<Arc<ProjectModel>> {
        let root = std::fs::canonicalize(path).with_context(|| format!("{} doesn't exist", path.display()))?;
        let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        if !root.join("package.json").exists() {
            bail!("{} has no package.json; open the folder that contains your Nest project", root.display());
        }
        if let Some(s) = self.inner.debug.lock().unwrap().take() {
            s.stop();
        }
        self.inner.procs.kill_all();
        let generation;
        {
            let mut st = self.inner.state.lock().unwrap();
            st.root = Some(root.clone());
            st.scanner = Scanner::new();
            st.pending.clear();
            st.undo.clear();
            st.redo.clear();
            st.generation += 1;
            generation = st.generation;
        }
        let model = self.rescan()?;
        self.start_watcher(&root, generation)?;
        self.inner.store.add_recent(&root)?;
        Ok(model)
    }

    pub fn rescan(&self) -> Result<Arc<ProjectModel>> {
        let mut st = self.inner.state.lock().unwrap();
        let root = st.root.clone().ok_or_else(|| anyhow!("No project is open"))?;
        let model = Arc::new(st.scanner.scan(&root));
        st.model = Some(model.clone());
        Ok(model)
    }

    fn rescan_and_emit(&self, changed: Vec<String>) {
        if let Ok(model) = self.rescan() {
            self.emit("project-updated", serde_json::to_value(&*model).unwrap_or(Value::Null));
        }
        if !changed.is_empty() {
            self.emit("files-changed", json!({ "paths": changed }));
        }
    }

    fn start_watcher(&self, root: &Path, generation: u64) -> Result<()> {
        let (tx, rx) = mpsc::channel::<Vec<String>>();
        let root_buf = root.to_path_buf();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            let paths: Vec<String> = ev
                .paths
                .iter()
                .filter_map(|p| p.strip_prefix(&root_buf).ok())
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .filter(|p| !p.split('/').any(|seg| IGNORED_DIRS.contains(&seg)))
                .collect();
            if !paths.is_empty() {
                let _ = tx.send(paths);
            }
        })?;
        watcher.watch(root, notify::RecursiveMode::Recursive)?;
        self.inner.state.lock().unwrap().watcher = Some(watcher);
        let me = self.clone();
        std::thread::spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut changed: Vec<String> = first;
                let deadline = Instant::now() + Duration::from_millis(120);
                while let Ok(more) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    changed.extend(more);
                }
                if me.inner.state.lock().unwrap().generation != generation {
                    return;
                }
                changed.sort();
                changed.dedup();
                me.rescan_and_emit(changed);
            }
        });
        Ok(())
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirEntry>> {
        let dir = self.path_in_root(rel)?;
        let mut out = vec![];
        for e in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            out.push(DirEntry { ignored: IGNORED_DIRS.contains(&name.as_str()), name, path, dir: is_dir });
        }
        out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    }

    fn list_files(&self) -> Result<Vec<String>> {
        let root = self.root()?;
        let mut out = vec![];
        let walker = ignore::WalkBuilder::new(&root)
            .hidden(false)
            .git_ignore(true)
            .require_git(false)
            .filter_entry(|e| !IGNORED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
            .build();
        for e in walker.flatten() {
            if e.file_type().is_some_and(|t| t.is_file()) {
                if let Ok(rel) = e.path().strip_prefix(&root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
            if out.len() > 20000 {
                break;
            }
        }
        out.sort();
        Ok(out)
    }

    fn apply_pending(&self, id: &str) -> Result<Value> {
        let cs = self.inner.state.lock().unwrap().pending.remove(id).ok_or_else(|| anyhow!("This preview expired; generate it again"))?;
        let root = self.root()?;
        changeset::apply(&root, &cs)?;
        {
            let mut st = self.inner.state.lock().unwrap();
            st.undo.push(cs.clone());
            st.redo.clear();
        }
        let paths: Vec<String> = cs.files.iter().map(|f| f.path.clone()).collect();
        self.rescan_and_emit(paths);
        if !cs.install.is_empty() || !cs.install_dev.is_empty() || cs.run_install {
            self.install_in_background(cs.install.clone(), cs.install_dev.clone(), cs.run_install);
        }
        let model = self.model()?;
        Ok(json!({ "changeset": cs, "model": &*model }))
    }

    fn install_in_background(&self, install: Vec<String>, dev: Vec<String>, plain: bool) {
        let me = self.clone();
        std::thread::spawn(move || {
            let res = me.install_packages(&install, &dev, plain);
            let ok = res.is_ok();
            me.emit("task-done", json!({ "id": "install", "ok": ok, "error": res.err().map(|e| e.to_string()) }));
            me.rescan_and_emit(vec!["package.json".into()]);
        });
    }

    /// The project's packages with the source files that import each one.
    fn package_list(&self) -> Result<pm::PackageList> {
        let root = self.root()?;
        let mut list = pm::list(&root);
        let files: Vec<String> = self.model()?.files.iter().map(|f| f.path.clone()).collect();
        let mut usages = pm::usages(&root, &files);
        for p in &mut list.packages {
            p.used_in = usages.remove(&p.name).unwrap_or_default();
        }
        Ok(list)
    }

    /// Runs one add (`specs`, then `dev_specs` as dev dependencies), remove or install in the
    /// background as process `packages`, then emits `task-done` and a rescan. Returns the
    /// command lines.
    fn run_packages(&self, action: PackageAction, specs: Vec<String>, dev: bool, dev_specs: Vec<String>) -> Result<Vec<String>> {
        if action != PackageAction::Install && specs.is_empty() && dev_specs.is_empty() {
            bail!("No packages given");
        }
        if let Some(bad) = specs.iter().chain(&dev_specs).find(|s| s.starts_with('-') || s.trim().is_empty()) {
            bail!("Not a package name: {bad}");
        }
        let root = self.root()?;
        let cmds = packages::commands_for(self.package_manager(), action, &specs, dev, &dev_specs);
        let shown: Vec<String> = cmds.iter().map(|c| pm::display(c)).collect();
        if self.inner.procs.is_running("install") || self.inner.packages_busy.swap(true, Ordering::SeqCst) {
            bail!("Another install is still running");
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let mut res = Ok(());
            for cmdline in cmds {
                let line = pm::display(&cmdline);
                me.emit("process-output", json!({ "id": "packages", "stream": "stdout", "text": format!("$ {line}") }));
                res = command(&cmdline[0], &cmdline[1..], &root, &[("FORCE_COLOR".into(), "1".into())])
                    .and_then(|c| me.inner.procs.run("packages", c))
                    .and_then(|(code, out)| if code == 0 { Ok(()) } else { Err(anyhow!("{line} failed:\n{}", tail(&out, 15))) });
                if res.is_err() {
                    break;
                }
            }
            me.inner.packages_busy.store(false, Ordering::SeqCst);
            me.emit("task-done", json!({ "id": "packages", "ok": res.is_ok(), "error": res.err().map(|e| format!("{e:#}")) }));
            me.rescan_and_emit(vec!["package.json".into()]);
        });
        Ok(shown)
    }

    fn install_packages(&self, install: &[String], dev: &[String], plain: bool) -> Result<()> {
        let root = self.root()?;
        let pmgr = self.package_manager();
        let mut cmds = vec![];
        if !install.is_empty() {
            cmds.push(pm::add(pmgr, install, false));
        }
        if !dev.is_empty() {
            cmds.push(pm::add(pmgr, dev, true));
        }
        if plain && cmds.is_empty() {
            cmds.push(pm::install(pmgr));
        }
        for c in cmds {
            self.emit("process-output", json!({ "id": "install", "stream": "stdout", "text": format!("$ {}", pm::display(&c)) }));
            let (code, out) = self.inner.procs.run("install", command(&c[0], &c[1..], &root, &[])?)?;
            if code != 0 {
                bail!("{} failed:\n{}", pm::display(&c), tail(&out, 15));
            }
        }
        Ok(())
    }

    fn undo(&self) -> Result<Value> {
        let root = self.root()?;
        let cs = self.inner.state.lock().unwrap().undo.pop().ok_or_else(|| anyhow!("Nothing to undo"))?;
        if let Err(e) = changeset::undo(&root, &cs) {
            self.inner.state.lock().unwrap().undo.push(cs);
            return Err(e);
        }
        let paths: Vec<String> = cs.files.iter().map(|f| f.path.clone()).collect();
        let label = cs.label.clone();
        self.inner.state.lock().unwrap().redo.push(cs);
        self.rescan_and_emit(paths);
        Ok(json!({ "label": label }))
    }

    fn redo(&self) -> Result<Value> {
        let root = self.root()?;
        let cs = self.inner.state.lock().unwrap().redo.pop().ok_or_else(|| anyhow!("Nothing to redo"))?;
        if let Err(e) = changeset::apply(&root, &cs) {
            self.inner.state.lock().unwrap().redo.push(cs);
            return Err(e);
        }
        let paths: Vec<String> = cs.files.iter().map(|f| f.path.clone()).collect();
        let label = cs.label.clone();
        self.inner.state.lock().unwrap().undo.push(cs);
        self.rescan_and_emit(paths);
        Ok(json!({ "label": label }))
    }

    fn save_profile(&self, cfg: &DbConfig, plan: Option<&DbPlan>) -> Result<()> {
        let root = self.root()?;
        let mut stored = cfg.clone();
        let has_password = !cfg.password.is_empty();
        if let Err(e) = store::set_password(&root, &cfg.name, &cfg.password) {
            self.emit("process-output", json!({ "id": "db-setup", "stream": "stderr", "text": format!("Could not save the password to the OS keychain: {e}") }));
        }
        stored.password = String::new();
        let profile = DbProfile {
            config: stored,
            has_password,
            module_file: plan.map(|p| p.module_file.clone()),
            container: plan.and_then(|p| p.docker.as_ref().map(|d| d.container.clone())),
            service: plan.and_then(|p| p.docker.as_ref().map(|d| d.service.clone())),
        };
        self.inner.store.update_project(&root, |d| match d.db_profiles.iter_mut().find(|p| p.config.name == cfg.name) {
            Some(p) => {
                let module_file = p.module_file.clone();
                let container = p.container.clone();
                let service = p.service.clone();
                *p = profile.clone();
                p.module_file = p.module_file.clone().or(module_file);
                p.container = p.container.clone().or(container);
                p.service = p.service.clone().or(service);
            }
            None => d.db_profiles.push(profile.clone()),
        })?;
        Ok(())
    }

    fn step(&self, step: &str, status: &str, detail: impl Into<String>) {
        self.emit("db-step", json!({ "step": step, "status": status, "detail": detail.into() }));
    }

    fn db_apply(&self, cfg: &DbConfig, changeset_id: Option<String>) -> Result<DbTestResult> {
        let root = self.root()?;
        let meta = read_meta(&root);
        let plan = db::plan(&root, &meta, &*self.model()?, cfg)?;
        // Files
        self.step("files", "running", "Writing module config and .env");
        let cs = match changeset_id.and_then(|id| self.inner.state.lock().unwrap().pending.remove(&id)) {
            Some(cs) => cs,
            None => plan.changeset.clone(),
        };
        changeset::apply(&root, &cs).map_err(|e| {
            self.step("files", "failed", e.to_string());
            e
        })?;
        {
            let mut st = self.inner.state.lock().unwrap();
            st.undo.push(cs.clone());
            st.redo.clear();
        }
        self.step("files", "done", plan.module_file.clone());
        // Packages
        let pkgs: Vec<String> = cs.install.iter().chain(cs.install_dev.iter()).cloned().collect();
        if !pkgs.is_empty() {
            self.step("packages", "running", pkgs.join(" "));
            if let Err(e) = self.install_packages(&cs.install, &cs.install_dev, false) {
                self.step("packages", "failed", e.to_string());
                return Err(e);
            }
            self.step("packages", "done", pkgs.join(" "));
        } else {
            self.step("packages", "done", "Already installed");
        }
        for c in &plan.post_commands {
            let (code, out) = self.inner.procs.run("db-setup", command(&c[0], &c[1..], &root, &[])?)?;
            if code != 0 {
                self.step("packages", "failed", tail(&out, 8));
                bail!("{} failed", pm::display(c));
            }
        }
        self.step("env", "done", format!("{} variables", plan.env_keys.len()));
        self.save_profile(cfg, Some(&plan))?;
        // Docker
        if let Some(d) = &plan.docker {
            self.step("docker", "running", format!("{} · port {}", d.container, d.port));
            let args = vec!["compose".to_string(), "up".to_string(), "-d".to_string(), d.service.clone()];
            let res = command("docker", &args, &root, &[]).and_then(|c| self.inner.procs.run("db-setup", c));
            match res {
                Ok((0, _)) => self.step("docker", "done", format!("{} · port {}", d.container, d.port)),
                Ok((_, out)) => {
                    self.step("docker", "failed", tail(&out, 6));
                    return Ok(DbTestResult { error: Some("Docker could not start the container. Is Docker Desktop running?".into()), ..Default::default() });
                }
                Err(e) => {
                    self.step("docker", "failed", e.to_string());
                    return Ok(DbTestResult { error: Some(e.to_string()), ..Default::default() });
                }
            }
        }
        self.rescan_and_emit(cs.files.iter().map(|f| f.path.clone()).collect());
        // Test, retrying while the container starts.
        self.step("test", "running", "Connecting");
        let deadline = Instant::now() + Duration::from_secs(if plan.docker.is_some() { 30 } else { 5 });
        let mut result = self.db_test(cfg);
        while !result.ok && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1500));
            result = self.db_test(cfg);
        }
        let detail = match (&result.version, &result.error) {
            (Some(v), _) => format!("{} · {:.0} ms", v.chars().take(40).collect::<String>(), result.latency_ms),
            (_, Some(e)) => e.clone(),
            _ => String::new(),
        };
        self.step("test", if result.ok { "done" } else { "failed" }, detail);
        Ok(result)
    }

    fn db_test(&self, cfg: &DbConfig) -> DbTestResult {
        let Ok(root) = self.root() else { return DbTestResult { error: Some("No project is open".into()), ..Default::default() } };
        let mut cfg = cfg.clone();
        if cfg.password.is_empty() {
            if let Some(p) = store::get_password(&root, &cfg.name) {
                cfg.password = p;
            }
        }
        let mut latency = 0.0;
        if cfg.database != db::Database::Sqlite {
            let started = Instant::now();
            let addr = format!("{}:{}", cfg.host, cfg.port);
            let addrs: Vec<_> = match addr.to_socket_addrs() {
                Ok(a) => a.collect(),
                Err(e) => return DbTestResult { error: Some(format!("Can't resolve {}: {e}", cfg.host)), ..Default::default() },
            };
            let ok = addrs.iter().any(|a| TcpStream::connect_timeout(a, Duration::from_secs(2)).is_ok());
            latency = started.elapsed().as_secs_f64() * 1000.0;
            if !ok {
                return DbTestResult { latency_ms: latency, error: Some(format!("Nothing is listening on {addr}")), ..Default::default() };
            }
        }
        let driver = match (cfg.orm, cfg.database) {
            (_, db::Database::Postgres) => "pg",
            (_, db::Database::Mysql) => "mysql2",
            (db::Orm::Mongoose, _) => "mongoose",
            (_, db::Database::Mongodb) => "mongodb",
            (_, db::Database::Sqlite) => "",
        };
        if !driver.is_empty() && !root.join("node_modules").join(driver).exists() {
            return DbTestResult { ok: true, latency_ms: latency, version: Some("Port is open".into()), tcp_only: true, error: None };
        }
        let (script, env) = db::driver_test_script(&cfg);
        let started = Instant::now();
        let out = command("node", &["-e".to_string(), script], &root, &env).and_then(|mut c| Ok(c.output()?));
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        match out {
            Ok(o) => {
                let text = String::from_utf8_lossy(&o.stdout);
                let line = text.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
                let v: Value = serde_json::from_str(line).unwrap_or(Value::Null);
                if v.get("ok").and_then(Value::as_bool) == Some(true) {
                    let version = v.get("version").and_then(Value::as_str).map(short_version);
                    DbTestResult { ok: true, latency_ms: if latency > 0.0 { latency } else { elapsed }, version, error: None, tcp_only: false }
                } else {
                    let err = v.get("error").and_then(Value::as_str).map(String::from).unwrap_or_else(|| tail(&String::from_utf8_lossy(&o.stderr), 4));
                    DbTestResult { ok: false, latency_ms: latency, version: None, error: Some(err), tcp_only: false }
                }
            }
            Err(e) => DbTestResult { error: Some(e.to_string()), ..Default::default() },
        }
    }

    fn run_tests(&self, e2e: bool, file: Option<String>, name: Option<String>, coverage: bool) -> Result<TestRun> {
        let root = self.root()?;
        let runner = test_runner(&root).ok_or_else(|| anyhow!("Neither Jest nor Vitest is installed in this project. Run an install first."))?;
        let tmp = std::env::temp_dir().join(format!("nestfactory-tests-{}", store::project_key(&root)));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        let results = tmp.join("results.json");
        let coverage_dir = tmp.join("coverage");
        let cov = coverage && !e2e && file.is_none();
        let mut args = vec![runner.bin.to_string_lossy().to_string()];
        if runner.vitest {
            // Vitest's JSON reporter writes Jest-compatible results.
            args.extend([
                "run".into(),
                "--reporter=default".into(),
                "--reporter=json".into(),
                format!("--outputFile.json={}", results.display()),
                "--includeTaskLocation".into(),
            ]);
            let has_cov = root.join("node_modules/@vitest/coverage-v8").exists() || root.join("node_modules/@vitest/coverage-istanbul").exists();
            if cov && has_cov {
                args.extend(["--coverage.enabled=true".into(), "--coverage.reporter=json".into(), format!("--coverage.reportsDirectory={}", coverage_dir.display())]);
            }
            if e2e {
                let cfg = if root.join("vitest.config.e2e.ts").exists() { "./vitest.config.e2e.ts" } else { "./test/vitest-e2e.config.ts" };
                args.extend(["--config".into(), cfg.into()]);
            }
        } else {
            args.extend(["--json".into(), format!("--outputFile={}", results.display()), "--testLocationInResults".into()]);
            if cov {
                args.extend(["--coverage".into(), "--coverageReporters=json".into(), format!("--coverageDirectory={}", coverage_dir.display())]);
            }
            if e2e {
                args.extend(["--config".into(), "./test/jest-e2e.json".into()]);
            }
        }
        if let Some(f) = &file {
            args.push(f.clone());
        }
        if let Some(n) = &name {
            args.extend(["-t".into(), n.clone()]);
        }
        let c = command("node", &args, &root, &[("FORCE_COLOR".into(), "1".into())])?;
        let (_code, out) = self.inner.procs.run("tests", c)?;
        let json = std::fs::read_to_string(&results).unwrap_or_default();
        let root_s = root.to_string_lossy().to_string();
        let mut run = testing::parse_jest(&root_s, &json, e2e);
        if json.is_empty() {
            run.error = Some(tail(&testing::strip_ansi(&out), 20));
        }
        if cov {
            if let Ok(c) = std::fs::read_to_string(coverage_dir.join("coverage-final.json")) {
                run.coverage = testing::parse_coverage(&root_s, &c);
            }
        }
        Ok(run)
    }

    fn typecheck(&self) -> Result<Vec<Problem>> {
        let root = self.root()?;
        let tsc = root.join("node_modules/typescript/bin/tsc");
        if !tsc.exists() {
            bail!("TypeScript isn't installed in this project. Run an install first.");
        }
        let args = vec![tsc.to_string_lossy().to_string(), "--noEmit".into(), "-p".into(), "tsconfig.json".into(), "--pretty".into(), "false".into()];
        let out = command("node", &args, &root, &[])?.output()?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok(testing::parse_tsc(&root.to_string_lossy(), &text))
    }

    fn endpoints(&self) -> Result<Vec<Endpoint>> {
        let model = self.model()?;
        let mut out: Vec<Endpoint> = model
            .routes
            .iter()
            .map(|r| Endpoint {
                method: r.method.clone(),
                path: r.path.clone(),
                summary: Some(format!("{}.{}", r.controller, r.handler)),
                tag: Some(r.controller.trim_end_matches("Controller").to_string()),
                source: "code".into(),
                controller: Some(r.controller.clone()),
                handler: Some(r.handler.clone()),
                file: Some(r.file.clone()),
                line: r.line,
                body_example: None,
            })
            .collect();
        if let Ok(sw) = http::swagger_endpoints(model.port) {
            for e in sw {
                match out.iter_mut().find(|x| x.method == e.method && x.path == e.path) {
                    Some(x) => {
                        x.source = "both".into();
                        if e.summary.is_some() {
                            x.summary = e.summary.clone();
                        }
                        x.body_example = e.body_example.clone();
                    }
                    None => out.push(e),
                }
            }
        }
        Ok(out)
    }

    fn wizard_step(&self, index: usize, status: &str, detail: impl Into<String>) {
        self.emit("wizard-step", json!({ "index": index, "status": status, "detail": detail.into() }));
    }

    fn wizard_run(&self, opts: &WizardOptions) -> Result<String> {
        wizard::validate(opts)?;
        let location = PathBuf::from(&opts.location);
        std::fs::create_dir_all(&location).with_context(|| format!("creating {}", location.display()))?;
        let project = location.join(&opts.name);
        let steps = wizard::plan(opts);
        for (i, step) in steps.iter().enumerate() {
            self.wizard_step(i, "running", step.title.clone());
            let res: Result<()> = match &step.command {
                Some(c) => {
                    let cwd = if step.in_project { &project } else { &location };
                    let mut c = c.clone();
                    if c[0] == "nest" && resolve_program("nest").is_none() {
                        c.splice(0..1, ["npx".to_string(), "--yes".to_string(), "@nestjs/cli@latest".to_string()]);
                    }
                    self.emit("process-output", json!({ "id": "wizard", "stream": "stdout", "text": format!("$ {}", pm::display(&c)) }));
                    let (code, out) = self.inner.procs.run("wizard", command(&c[0], &c[1..], cwd, &[])?)?;
                    if code == 0 { Ok(()) } else { Err(anyhow!("{} failed:\n{}", pm::display(&c), tail(&testing::strip_ansi(&out), 12))) }
                }
                None => wizard::starter_changeset(&project, opts).and_then(|cs| changeset::apply(&project, &cs)),
            };
            match res {
                Ok(()) => self.wizard_step(i, "done", ""),
                Err(e) => {
                    self.wizard_step(i, "failed", e.to_string());
                    return Err(e);
                }
            }
        }
        Ok(project.to_string_lossy().to_string())
    }
}

pub struct TestRunner {
    pub vitest: bool,
    pub bin: PathBuf,
}

/// The project's test runner: Vitest when its test script uses it (Nest 12 default),
/// otherwise Jest (Nest 11 and earlier).
pub fn test_runner(root: &Path) -> Option<TestRunner> {
    let jest = root.join("node_modules/jest/bin/jest.js");
    let vitest = root.join("node_modules/vitest/vitest.mjs");
    let script = nestfactory_core::project::read_json(&root.join("package.json"))
        .and_then(|p| p.pointer("/scripts/test").and_then(Value::as_str).map(String::from))
        .unwrap_or_default();
    if vitest.exists() && (script.contains("vitest") || !jest.exists()) {
        return Some(TestRunner { vitest: true, bin: vitest });
    }
    if jest.exists() {
        return Some(TestRunner { vitest: false, bin: jest });
    }
    None
}

fn short_version(v: &str) -> String {
    // "PostgreSQL 16.4 (Debian ...) on x86_64..." → "PostgreSQL 16.4"
    let words: Vec<&str> = v.split_whitespace().collect();
    if words.len() >= 2 && words[0] == "PostgreSQL" {
        return format!("{} {}", words[0], words[1]);
    }
    v.to_string()
}

pub fn tail(s: &str, lines: usize) -> String {
    let v: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    v[v.len().saturating_sub(lines)..].join("\n")
}

fn version_of(program: &str, args: &[&str]) -> Option<String> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let out = command(program, &args, &std::env::temp_dir(), &[]).ok()?.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then(|| s.lines().next().unwrap_or("").to_string())
}

pub fn toolchain() -> Toolchain {
    let pms = [PackageManager::Npm, PackageManager::Pnpm, PackageManager::Yarn, PackageManager::Bun];
    Toolchain {
        node: version_of("node", &["--version"]),
        nest: version_of("nest", &["--version"]),
        docker: version_of("docker", &["--version"]),
        git: version_of("git", &["--version"]),
        package_managers: pms.into_iter().filter(|p| resolve_program(pm::program(*p)).is_some()).collect(),
    }
}

pub fn docker_status(container: &str) -> DockerStatus {
    let args: Vec<String> = ["ps", "-a", "--filter", &format!("name=^/{container}$"), "--format", "{{.Status}}"].iter().map(|s| s.to_string()).collect();
    let Ok(mut c) = command("docker", &args, &std::env::temp_dir(), &[]) else { return DockerStatus::default() };
    match c.output() {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            DockerStatus { available: true, exists: !s.is_empty(), running: s.starts_with("Up"), status: s }
        }
        Ok(o) => DockerStatus { available: false, status: tail(&String::from_utf8_lossy(&o.stderr), 2), ..Default::default() },
        Err(e) => DockerStatus { available: false, status: e.to_string(), ..Default::default() },
    }
}
