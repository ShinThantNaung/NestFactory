//! The project model shared with the UI. Every type here derives `TS`, so `cargo test`
//! regenerates `ui/src/bindings/*.ts` and the Rust/TypeScript boundary stays typed.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ClassKind {
    Module,
    Controller,
    Service,
    Provider,
    Entity,
    Schema,
    Dto,
    Guard,
    Pipe,
    Interceptor,
    Filter,
    Middleware,
    Gateway,
    Resolver,
    Strategy,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileKind {
    Module,
    Controller,
    Service,
    Entity,
    Dto,
    Spec,
    Guard,
    Pipe,
    Interceptor,
    Filter,
    Middleware,
    Gateway,
    Resolver,
    Main,
    Other,
}

impl FileKind {
    pub fn from_path(path: &str) -> FileKind {
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.ends_with(".spec.ts") || name.ends_with(".e2e-spec.ts") || name.ends_with(".test.ts") {
            return FileKind::Spec;
        }
        let table = [
            (".module.ts", FileKind::Module),
            (".controller.ts", FileKind::Controller),
            (".service.ts", FileKind::Service),
            (".entity.ts", FileKind::Entity),
            (".schema.ts", FileKind::Entity),
            (".model.ts", FileKind::Entity),
            (".dto.ts", FileKind::Dto),
            (".input.ts", FileKind::Dto),
            (".guard.ts", FileKind::Guard),
            (".pipe.ts", FileKind::Pipe),
            (".interceptor.ts", FileKind::Interceptor),
            (".filter.ts", FileKind::Filter),
            (".middleware.ts", FileKind::Middleware),
            (".gateway.ts", FileKind::Gateway),
            (".resolver.ts", FileKind::Resolver),
        ];
        for (suffix, kind) in table {
            if name.ends_with(suffix) {
                return kind;
            }
        }
        if name == "main.ts" {
            return FileKind::Main;
        }
        FileKind::Other
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectModel {
    pub root: String,
    pub name: String,
    pub nest_version: Option<String>,
    pub typescript_version: Option<String>,
    pub package_manager: PackageManager,
    pub monorepo: bool,
    pub esm: bool,
    pub source_root: String,
    pub scripts: Vec<String>,
    pub dependencies: Vec<String>,
    pub modules: Vec<ModuleInfo>,
    pub classes: Vec<ClassInfo>,
    pub routes: Vec<RouteInfo>,
    pub root_modules: Vec<String>,
    pub global_prefix: Option<String>,
    pub port: u32,
    pub files: Vec<SourceFileInfo>,
    pub graph: Graph,
    pub provider_graph: Graph,
    pub diagnostics: Vec<Diagnostic>,
    pub scan_ms: f64,
    pub git_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceFileInfo {
    pub path: String,
    pub kind: FileKind,
    pub module: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleInfo {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub global: bool,
    /// The metadata uses spreads or computed entries, so analysis may be incomplete.
    pub dynamic: bool,
    pub imports: Vec<ModuleImport>,
    pub providers: Vec<ProviderEntry>,
    pub controllers: Vec<String>,
    pub exports: Vec<String>,
    pub orm: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleImport {
    pub name: String,
    /// What the entry looks like in source, e.g. `TypeOrmModule.forFeature`.
    pub display: String,
    pub external: bool,
    pub package: Option<String>,
    pub forward_ref: bool,
    pub method: Option<String>,
    /// Feature names passed to `forFeature([...])`.
    pub features: Vec<String>,
    pub global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderEntry {
    pub token: String,
    pub class_name: Option<String>,
    pub style: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClassInfo {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub kind: ClassKind,
    pub exported: bool,
    pub decorators: Vec<String>,
    pub injects: Vec<Injection>,
    pub prefix: Option<String>,
    pub transport: Option<String>,
    pub modules: Vec<String>,
    pub extends: Option<String>,
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TokenKind {
    Class,
    Repository,
    Model,
    Token,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum InjectionStatus {
    Resolved,
    Missing,
    Unchecked,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Injection {
    pub param: String,
    pub token: String,
    pub token_kind: TokenKind,
    pub type_text: String,
    pub line: u32,
    pub optional: bool,
    pub forward_ref: bool,
    pub status: InjectionStatus,
    /// Where the token comes from: the module that provides or exports it.
    pub via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FieldInfo {
    pub name: String,
    pub type_text: String,
    pub optional: bool,
    pub decorators: Vec<String>,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RouteInfo {
    pub method: String,
    pub path: String,
    pub controller: String,
    pub handler: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NodeKind {
    Module,
    External,
    Provider,
    Controller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NodeStatus {
    Ok,
    Warning,
    Error,
    Orphan,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub sublabel: String,
    pub kind: NodeKind,
    pub file: Option<String>,
    pub line: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub status: NodeStatus,
    pub root: bool,
    pub global: bool,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EdgeKind {
    Import,
    Cycle,
    Missing,
    Inject,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub forward_ref: bool,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiagnosticCode {
    CircularImport,
    CircularInjection,
    MissingProvider,
    ModuleNotImported,
    ControllerNotRegistered,
    ProviderNotRegistered,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Diagnostic {
    pub id: String,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub title: String,
    pub subject: String,
    pub detail: String,
    pub file: Option<String>,
    pub line: u32,
    pub modules: Vec<String>,
    pub fixes: Vec<Fix>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Fix {
    pub label: String,
    pub action: FixAction,
}

/// A wiring change the IDE can preview as a diff and apply as one undoable step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum FixAction {
    /// Add `module` to `into`'s imports. `package` is set for library modules.
    ImportModule { module: String, into: String, package: Option<String> },
    /// Add a provider class to a module's providers.
    AddProvider { provider: String, module: String },
    /// Add a controller class to a module's controllers.
    AddController { controller: String, module: String },
    /// Export `provider` from `from`, then import `from` into `into`.
    ExportAndImport { provider: String, from: String, into: String },
    /// Wrap the imports between these modules in `forwardRef(() => X)`.
    ApplyForwardRef { modules: Vec<String> },
    /// Register an entity with `TypeOrmModule.forFeature([...])` (or the ORM's equivalent).
    AddFeature { orm_module: String, entity: String, module: String },
}
