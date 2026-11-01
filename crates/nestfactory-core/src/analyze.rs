//! Builds the project model from parsed files: modules, providers, injection, routes, the
//! module graph and the diagnostics that would otherwise surface as runtime DI errors.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::layout::layered_layout;
use crate::model::*;
use crate::project::{normalize_path, parent_dir, ProjectMeta};
use crate::ts::lexer::Kind;
use crate::ts::parse::{ClassDecl, Decorator, Parsed};

pub struct FileEntry {
    pub rel: String,
    pub parsed: Arc<Parsed>,
}

#[derive(Debug, Clone)]
enum Target {
    Local { file: usize, name: String },
    Package { pkg: String },
}

/// A class referenced from a module's metadata array.
#[derive(Debug, Clone)]
enum Item {
    Ident(String),
    Call { base: String, method: String, args: (usize, usize) },
    ForwardRef(String),
    Object(usize),
    Str(String),
    Other,
}

const BUILTIN_TOKENS: &[&str] = &[
    "Reflector", "ModuleRef", "HttpAdapterHost", "LazyModuleLoader", "ApplicationConfig",
    "ModulesContainer", "REQUEST", "INQUIRER", "ExternalContextCreator",
];

/// Library classes that need their module imported, keyed by class → (module, package).
const PACKAGE_CLASSES: &[(&str, &str, &str)] = &[
    ("ConfigService", "ConfigModule", "@nestjs/config"),
    ("JwtService", "JwtModule", "@nestjs/jwt"),
    ("HttpService", "HttpModule", "@nestjs/axios"),
    ("HealthCheckService", "TerminusModule", "@nestjs/terminus"),
    ("TypeOrmHealthIndicator", "TerminusModule", "@nestjs/terminus"),
    ("HttpHealthIndicator", "TerminusModule", "@nestjs/terminus"),
    ("MongooseHealthIndicator", "TerminusModule", "@nestjs/terminus"),
    ("PrismaHealthIndicator", "TerminusModule", "@nestjs/terminus"),
    ("DiskHealthIndicator", "TerminusModule", "@nestjs/terminus"),
    ("MemoryHealthIndicator", "TerminusModule", "@nestjs/terminus"),
    ("CommandBus", "CqrsModule", "@nestjs/cqrs"),
    ("QueryBus", "CqrsModule", "@nestjs/cqrs"),
    ("EventBus", "CqrsModule", "@nestjs/cqrs"),
    ("SchedulerRegistry", "ScheduleModule", "@nestjs/schedule"),
    ("EventEmitter2", "EventEmitterModule", "@nestjs/event-emitter"),
];

const ORM_MODULES: &[(&str, &str)] = &[
    ("TypeOrmModule", "TypeORM"),
    ("MongooseModule", "Mongoose"),
    ("SequelizeModule", "Sequelize"),
    ("MikroOrmModule", "MikroORM"),
    ("PrismaModule", "Prisma"),
];

struct ModuleData {
    info: ModuleInfo,
    file: usize,
    provided: Vec<String>,
    provider_classes: Vec<String>,
    exports: Vec<String>,
    local_imports: Vec<(usize, bool)>,
    external_tokens: Vec<String>,
}

struct Ctx<'a> {
    meta: &'a ProjectMeta,
    files: &'a [FileEntry],
    file_index: HashMap<&'a str, usize>,
    imports: Vec<HashMap<String, Target>>,
    classes_by_name: HashMap<String, Vec<(usize, usize)>>,
}

impl<'a> Ctx<'a> {
    fn new(meta: &'a ProjectMeta, files: &'a [FileEntry]) -> Self {
        let file_index: HashMap<&str, usize> =
            files.iter().enumerate().map(|(i, f)| (f.rel.as_str(), i)).collect();
        let mut ctx = Ctx { meta, files, file_index, imports: vec![], classes_by_name: HashMap::new() };
        let mut imports = Vec::with_capacity(files.len());
        for f in files {
            let mut map = HashMap::new();
            for imp in &f.parsed.imports {
                let target = ctx.resolve_specifier(&f.rel, &imp.source);
                for (imported, local) in &imp.names {
                    let t = match &target {
                        Some(file) => Target::Local { file: *file, name: imported.clone() },
                        None => Target::Package { pkg: package_name(&imp.source) },
                    };
                    map.insert(local.clone(), t);
                }
            }
            imports.push(map);
        }
        ctx.imports = imports;
        for (fi, f) in files.iter().enumerate() {
            if is_spec(&f.rel) {
                continue;
            }
            for (ci, c) in f.parsed.classes.iter().enumerate() {
                ctx.classes_by_name.entry(c.name.clone()).or_default().push((fi, ci));
            }
        }
        ctx
    }

    fn resolve_specifier(&self, from: &str, spec: &str) -> Option<usize> {
        let mut bases = vec![];
        if spec.starts_with('.') {
            bases.push(normalize_path(&format!("{}/{}", parent_dir(from), spec)));
        } else {
            for (pattern, targets) in &self.meta.path_aliases {
                if let Some(prefix) = pattern.strip_suffix('*') {
                    if let Some(rest) = spec.strip_prefix(prefix) {
                        for t in targets {
                            bases.push(normalize_path(&t.replace('*', rest)));
                        }
                    }
                } else if pattern == spec {
                    for t in targets {
                        bases.push(normalize_path(t));
                    }
                }
            }
        }
        for base in bases {
            let base = base.strip_suffix(".js").unwrap_or(&base).to_string();
            for candidate in [format!("{base}.ts"), format!("{base}/index.ts"), base.clone()] {
                if let Some(&i) = self.file_index.get(candidate.as_str()) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Resolves a class name as seen from `file`.
    fn resolve_class(&self, file: usize, name: &str) -> Option<(usize, usize)> {
        if let Some(t) = self.imports[file].get(name) {
            match t {
                Target::Local { file: f, name: n } => {
                    if let Some(ci) = self.files[*f].parsed.classes.iter().position(|c| &c.name == n) {
                        return Some((*f, ci));
                    }
                    // Re-exported through an index file: fall back to a unique name match.
                    return self.unique_class(n);
                }
                Target::Package { .. } => return None,
            }
        }
        if let Some(ci) = self.files[file].parsed.classes.iter().position(|c| c.name == name) {
            return Some((file, ci));
        }
        self.unique_class(name)
    }

    fn unique_class(&self, name: &str) -> Option<(usize, usize)> {
        match self.classes_by_name.get(name) {
            Some(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    }

    fn package_of(&self, file: usize, name: &str) -> Option<String> {
        match self.imports[file].get(name) {
            Some(Target::Package { pkg }) => Some(pkg.clone()),
            _ => None,
        }
    }
}

fn package_name(spec: &str) -> String {
    let mut parts = spec.split('/');
    match parts.next() {
        Some(scope) if scope.starts_with('@') => match parts.next() {
            Some(n) => format!("{scope}/{n}"),
            None => scope.to_string(),
        },
        Some(n) => n.to_string(),
        None => spec.to_string(),
    }
}

pub fn is_spec(rel: &str) -> bool {
    matches!(FileKind::from_path(rel), FileKind::Spec)
}

fn has_decorator(c: &ClassDecl, name: &str) -> bool {
    c.decorators.iter().any(|d| d.name == name)
}

fn decorator<'c>(c: &'c ClassDecl, name: &str) -> Option<&'c Decorator> {
    c.decorators.iter().find(|d| d.name == name)
}

fn classify(c: &ClassDecl, rel: &str) -> ClassKind {
    let file_kind = FileKind::from_path(rel);
    let implements = |n: &str| c.implements.iter().any(|i| i == n);
    if has_decorator(c, "Module") {
        return ClassKind::Module;
    }
    if has_decorator(c, "Controller") {
        return ClassKind::Controller;
    }
    if has_decorator(c, "Resolver") {
        return ClassKind::Resolver;
    }
    if has_decorator(c, "WebSocketGateway") {
        return ClassKind::Gateway;
    }
    if has_decorator(c, "Catch") {
        return ClassKind::Filter;
    }
    if has_decorator(c, "Entity") || has_decorator(c, "Table") || has_decorator(c, "ObjectType") {
        return ClassKind::Entity;
    }
    if has_decorator(c, "Schema") {
        return ClassKind::Schema;
    }
    if has_decorator(c, "InputType") || has_decorator(c, "ArgsType") {
        return ClassKind::Dto;
    }
    if has_decorator(c, "Injectable") {
        if implements("CanActivate") || file_kind == FileKind::Guard {
            return ClassKind::Guard;
        }
        if implements("PipeTransform") || file_kind == FileKind::Pipe {
            return ClassKind::Pipe;
        }
        if implements("NestInterceptor") || file_kind == FileKind::Interceptor {
            return ClassKind::Interceptor;
        }
        if implements("NestMiddleware") || file_kind == FileKind::Middleware {
            return ClassKind::Middleware;
        }
        if c.extends.as_deref().is_some_and(|e| e.starts_with("PassportStrategy") || e.starts_with("AuthGuard")) {
            return if e_is_guard(c) { ClassKind::Guard } else { ClassKind::Strategy };
        }
        if c.name.ends_with("Service") || file_kind == FileKind::Service {
            return ClassKind::Service;
        }
        return ClassKind::Provider;
    }
    if c.name.ends_with("Dto") || file_kind == FileKind::Dto {
        return ClassKind::Dto;
    }
    if file_kind == FileKind::Entity {
        return ClassKind::Entity;
    }
    ClassKind::Other
}

fn e_is_guard(c: &ClassDecl) -> bool {
    c.extends.as_deref().is_some_and(|e| e.starts_with("AuthGuard"))
}

fn read_item(p: &Parsed, (s, e): (usize, usize)) -> Item {
    let n = e - s;
    if n == 1 {
        return match p.kind(s) {
            Some(Kind::Ident) => Item::Ident(p.text(s).to_string()),
            Some(Kind::Str) => Item::Str(p.string_value(s).unwrap_or_default()),
            _ => Item::Other,
        };
    }
    if p.is(s, "{") && p.matching(s) == Some(e - 1) {
        return Item::Object(s);
    }
    // forwardRef(() => X)
    if p.is(s, "forwardRef") && p.is(s + 1, "(") {
        let close = p.matching(s + 1).unwrap_or(e - 1);
        let mut k = s + 2;
        while k < close {
            if p.is(k, "=>") && p.kind(k + 1) == Some(Kind::Ident) {
                return Item::ForwardRef(p.text(k + 1).to_string());
            }
            k += 1;
        }
        return Item::Other;
    }
    // X.method(args)
    if p.kind(s) == Some(Kind::Ident) && p.is(s + 1, ".") && p.kind(s + 2) == Some(Kind::Ident) && p.is(s + 3, "(") {
        if let Some(close) = p.matching(s + 3) {
            return Item::Call {
                base: p.text(s).to_string(),
                method: p.text(s + 2).to_string(),
                args: (s + 3, close),
            };
        }
    }
    Item::Other
}

/// Names passed to `forFeature([A, B])` or `forFeature([{ name: A.name, schema }])`.
fn feature_names(p: &Parsed, args: (usize, usize)) -> Vec<String> {
    let mut out = vec![];
    let items = p.split_items(args.0, args.1);
    let Some(&(s, _)) = items.first() else { return out };
    if !p.is(s, "[") {
        return out;
    }
    let Some(close) = p.matching(s) else { return out };
    for item in p.split_items(s, close) {
        match read_item(p, item) {
            Item::Ident(n) => out.push(n),
            Item::Object(open) => {
                for prop in p.object_props(open) {
                    if prop.key == "name" {
                        let (vs, ve) = prop.value;
                        if p.kind(vs) == Some(Kind::Ident) {
                            out.push(p.text(vs).to_string());
                        } else if let Some(sv) = p.string_value(vs) {
                            let _ = ve;
                            out.push(sv);
                        }
                    }
                }
            }
            Item::Str(sv) => out.push(sv),
            _ => {}
        }
    }
    out
}

fn object_flag(p: &Parsed, args: (usize, usize), key: &str) -> bool {
    for (s, _) in p.split_items(args.0, args.1) {
        if p.is(s, "{") {
            for prop in p.object_props(s) {
                if prop.key == key && p.is(prop.value.0, "true") {
                    return true;
                }
            }
        }
    }
    false
}

/// Tokens a library module makes available to its importer, and globally.
fn external_tokens(name: &str, method: Option<&str>, features: &[String], global_flag: bool) -> (Vec<String>, Vec<String>) {
    let root = matches!(method, Some("forRoot" | "forRootAsync"));
    let feat = |tpl: &str| features.iter().map(|f| tpl.replace("{}", f)).collect::<Vec<_>>();
    let (local, global): (Vec<String>, Vec<String>) = match name {
        "ConfigModule" => (vec!["ConfigService".into()], if global_flag { vec!["ConfigService".into()] } else { vec![] }),
        "TypeOrmModule" if root => (vec![], vec!["DataSource".into(), "EntityManager".into()]),
        "TypeOrmModule" => (feat("Repository<{}>"), vec![]),
        "MikroOrmModule" if root => (vec![], vec!["MikroORM".into(), "EntityManager".into()]),
        "MikroOrmModule" => (feat("Repository<{}>"), vec![]),
        "MongooseModule" if root => (vec![], vec!["Connection".into()]),
        "MongooseModule" => (feat("Model<{}>"), vec![]),
        "SequelizeModule" if root => (vec![], vec!["Sequelize".into()]),
        "SequelizeModule" => (feat("Model<{}>"), vec![]),
        "JwtModule" => (vec!["JwtService".into()], if global_flag { vec!["JwtService".into()] } else { vec![] }),
        "HttpModule" => (vec!["HttpService".into()], vec![]),
        "TerminusModule" => (
            PACKAGE_CLASSES.iter().filter(|(_, m, _)| *m == "TerminusModule").map(|(c, _, _)| c.to_string()).collect(),
            vec![],
        ),
        "CqrsModule" => {
            let t: Vec<String> = ["CommandBus", "QueryBus", "EventBus", "EventPublisher"].iter().map(|s| s.to_string()).collect();
            (t.clone(), if root { t } else { vec![] })
        }
        "ScheduleModule" => (vec![], vec!["SchedulerRegistry".into()]),
        "EventEmitterModule" => (vec![], vec!["EventEmitter2".into()]),
        "CacheModule" => (vec!["CACHE_MANAGER".into()], if global_flag { vec!["CACHE_MANAGER".into()] } else { vec![] }),
        "BullModule" => (feat("Queue<{}>"), vec![]),
        "ClientsModule" => (features.iter().map(|f| format!("'{f}'")).collect(), vec![]),
        _ => (vec![], vec![]),
    };
    (local, global)
}

/// The injection token for a constructor parameter.
fn injection_token(p: &Parsed, param: &crate::ts::parse::Param) -> (String, TokenKind, bool, bool) {
    let mut forward = false;
    let optional = param.decorators.iter().any(|d| d.name == "Optional") || param.optional;
    for d in &param.decorators {
        let arg = p.decorator_arg(d, 0);
        match d.name.as_str() {
            "Inject" => {
                if let Some(a) = arg {
                    match read_item(p, a) {
                        Item::Ident(n) => return (n, kind_of_ident(p, a.0), false, optional),
                        Item::Str(s) => return (format!("'{s}'"), TokenKind::Token, false, optional),
                        Item::ForwardRef(n) => {
                            forward = true;
                            return (n, TokenKind::Class, forward, optional);
                        }
                        _ => return (p.slice(a.0, a.1).to_string(), TokenKind::Unknown, false, optional),
                    }
                }
            }
            "InjectRepository" => {
                if let Some(a) = arg {
                    return (format!("Repository<{}>", p.text(a.0)), TokenKind::Repository, false, optional);
                }
            }
            "InjectModel" => {
                if let Some(a) = arg {
                    let name = p.string_value(a.0).unwrap_or_else(|| p.text(a.0).to_string());
                    return (format!("Model<{name}>"), TokenKind::Model, false, optional);
                }
            }
            "InjectDataSource" | "InjectConnection" => {
                let t = if param.type_name.as_deref() == Some("Connection") { "Connection" } else { "DataSource" };
                return (t.into(), TokenKind::Class, false, optional);
            }
            "InjectEntityManager" => return ("EntityManager".into(), TokenKind::Class, false, optional),
            "InjectQueue" => {
                if let Some(a) = arg {
                    return (format!("Queue<{}>", p.string_value(a.0).unwrap_or_default()), TokenKind::Token, false, optional);
                }
            }
            _ => {}
        }
    }
    match &param.type_name {
        Some(t) if t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) => (t.clone(), TokenKind::Class, forward, optional),
        _ => (param.type_text.clone(), TokenKind::Unknown, forward, optional),
    }
}

fn kind_of_ident(p: &Parsed, tok: usize) -> TokenKind {
    let t = p.text(tok);
    if t.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) {
        TokenKind::Token
    } else {
        TokenKind::Class
    }
}

pub fn analyze(meta: &ProjectMeta, files: &[FileEntry]) -> ProjectModel {
    let started = std::time::Instant::now();
    let ctx = Ctx::new(meta, files);

    // ---- modules --------------------------------------------------------------------------
    let mut modules: Vec<ModuleData> = vec![];
    let mut module_by_class: HashMap<(usize, usize), usize> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        if is_spec(&f.rel) {
            continue;
        }
        for (ci, c) in f.parsed.classes.iter().enumerate() {
            let Some(d) = decorator(c, "Module") else { continue };
            module_by_class.insert((fi, ci), modules.len());
            modules.push(ModuleData {
                info: ModuleInfo {
                    name: c.name.clone(),
                    file: f.rel.clone(),
                    line: c.line,
                    global: has_decorator(c, "Global"),
                    dynamic: false,
                    imports: vec![],
                    providers: vec![],
                    controllers: vec![],
                    exports: vec![],
                    orm: None,
                },
                file: fi,
                provided: vec![],
                provider_classes: vec![],
                exports: vec![],
                local_imports: vec![],
                external_tokens: vec![],
            });
            let _ = d;
        }
    }

    let mut global_tokens: BTreeSet<String> = BUILTIN_TOKENS.iter().map(|s| s.to_string()).collect();
    let mut global_token_source: HashMap<String, String> = HashMap::new();
    let module_index_by_class = module_by_class.clone();

    for mi in 0..modules.len() {
        let fi = modules[mi].file;
        let p = &files[fi].parsed;
        let class_ci = p.classes.iter().position(|c| c.name == modules[mi].info.name).unwrap();
        let c = &p.classes[class_ci];
        let d = decorator(c, "Module").unwrap();
        let Some((s, _)) = p.decorator_arg(d, 0) else { continue };
        if !p.is(s, "{") {
            modules[mi].info.dynamic = true;
            continue;
        }
        let mut dynamic = false;
        for prop in p.object_props(s) {
            if prop.spread {
                dynamic = true;
                continue;
            }
            let (vs, ve) = prop.value;
            if !p.is(vs, "[") || p.matching(vs) != Some(ve - 1) {
                dynamic = true;
                continue;
            }
            let items = p.split_items(vs, ve - 1);
            for item in items {
                if p.is(item.0, "...") {
                    dynamic = true;
                    continue;
                }
                let it = read_item(p, item);
                match prop.key.as_str() {
                    "imports" => {
                        let (name, method, args, forward) = match &it {
                            Item::Ident(n) => (n.clone(), None, None, false),
                            Item::ForwardRef(n) => (n.clone(), None, None, true),
                            Item::Call { base, method, args } => (base.clone(), Some(method.clone()), Some(*args), false),
                            _ => {
                                dynamic = true;
                                continue;
                            }
                        };
                        let local = ctx
                            .resolve_class(fi, &name)
                            .and_then(|cref| module_index_by_class.get(&cref).copied());
                        let features = args.map(|a| feature_names(p, a)).unwrap_or_default();
                        let global_flag = args.is_some_and(|a| object_flag(p, a, "isGlobal") || object_flag(p, a, "global"));
                        let display = match &method {
                            Some(m) => format!("{name}.{m}"),
                            None => name.clone(),
                        };
                        if let Some(target) = local {
                            modules[mi].local_imports.push((target, forward));
                            modules[mi].info.imports.push(ModuleImport {
                                name,
                                display,
                                external: false,
                                package: None,
                                forward_ref: forward,
                                method,
                                features,
                                global: false,
                            });
                        } else {
                            let (toks, globals) = external_tokens(&name, method.as_deref(), &features, global_flag);
                            for g in &globals {
                                global_tokens.insert(g.clone());
                                global_token_source.entry(g.clone()).or_insert_with(|| name.clone());
                            }
                            modules[mi].external_tokens.extend(toks);
                            if modules[mi].info.orm.is_none() {
                                if let Some((_, orm)) = ORM_MODULES.iter().find(|(m, _)| *m == name) {
                                    modules[mi].info.orm = Some(orm.to_string());
                                }
                            }
                            let package = ctx.package_of(fi, &name);
                            modules[mi].info.imports.push(ModuleImport {
                                name,
                                display,
                                external: true,
                                package,
                                forward_ref: forward,
                                method,
                                features,
                                global: !globals.is_empty(),
                            });
                        }
                    }
                    "providers" => match it {
                        Item::Ident(n) => {
                            modules[mi].provided.push(n.clone());
                            modules[mi].provider_classes.push(n.clone());
                            modules[mi].info.providers.push(ProviderEntry { token: n.clone(), class_name: Some(n), style: "class".into() });
                        }
                        Item::Object(open) => {
                            let props = p.object_props(open);
                            let mut token = None;
                            let mut class_name = None;
                            let mut style = "value".to_string();
                            for pr in &props {
                                let (a, b) = pr.value;
                                match pr.key.as_str() {
                                    "provide" => {
                                        token = Some(match p.string_value(a) {
                                            Some(sv) if b == a + 1 => format!("'{sv}'"),
                                            _ => p.slice(a, b).to_string(),
                                        })
                                    }
                                    "useClass" => {
                                        class_name = Some(p.slice(a, b).to_string());
                                        style = "class".into();
                                    }
                                    "useFactory" => style = "factory".into(),
                                    "useExisting" => style = "existing".into(),
                                    _ => {}
                                }
                            }
                            if let Some(t) = token {
                                modules[mi].provided.push(t.clone());
                                if let Some(cn) = &class_name {
                                    modules[mi].provider_classes.push(cn.clone());
                                }
                                modules[mi].info.providers.push(ProviderEntry { token: t, class_name, style });
                            } else {
                                dynamic = true;
                            }
                        }
                        _ => dynamic = true,
                    },
                    "controllers" => match it {
                        Item::Ident(n) => modules[mi].info.controllers.push(n),
                        _ => dynamic = true,
                    },
                    "exports" => match it {
                        Item::Ident(n) => {
                            modules[mi].exports.push(n.clone());
                            modules[mi].info.exports.push(n);
                        }
                        Item::Str(sv) => {
                            modules[mi].exports.push(format!("'{sv}'"));
                            modules[mi].info.exports.push(format!("'{sv}'"));
                        }
                        Item::Call { base, method, .. } => {
                            modules[mi].exports.push(base.clone());
                            modules[mi].info.exports.push(format!("{base}.{method}"));
                        }
                        _ => dynamic = true,
                    },
                    _ => {}
                }
            }
        }
        modules[mi].info.dynamic = dynamic;
    }

    // ---- export sets ----------------------------------------------------------------------
    let module_by_name: HashMap<String, Vec<usize>> = {
        let mut m: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, md) in modules.iter().enumerate() {
            m.entry(md.info.name.clone()).or_default().push(i);
        }
        m
    };
    let export_sets: Vec<BTreeSet<String>> = (0..modules.len())
        .map(|i| {
            let mut out = BTreeSet::new();
            let mut seen = HashSet::new();
            collect_exports(&modules, &module_by_name, i, &mut out, &mut seen);
            out
        })
        .collect();
    for (i, md) in modules.iter().enumerate() {
        if md.info.global {
            for t in &export_sets[i] {
                global_tokens.insert(t.clone());
                global_token_source.entry(t.clone()).or_insert_with(|| md.info.name.clone());
            }
        }
    }

    // token → modules providing it / exporting it
    let mut providers_of: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, md) in modules.iter().enumerate() {
        for t in &md.provided {
            providers_of.entry(t.clone()).or_default().push(i);
        }
    }
    let mut exporters_of: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, set) in export_sets.iter().enumerate() {
        for t in set {
            exporters_of.entry(t.clone()).or_default().push(i);
        }
    }

    // available tokens per module, with where each comes from
    let available: Vec<HashMap<String, String>> = modules
        .iter()
        .map(|md| {
            let mut m: HashMap<String, String> = HashMap::new();
            for t in &md.provided {
                m.insert(t.clone(), md.info.name.clone());
            }
            for (target, _) in &md.local_imports {
                for t in &export_sets[*target] {
                    m.entry(t.clone()).or_insert_with(|| modules[*target].info.name.clone());
                }
            }
            for imp in md.info.imports.iter().filter(|i| i.external) {
                let (toks, _) = external_tokens(&imp.name, imp.method.as_deref(), &imp.features, false);
                for t in toks {
                    m.entry(t).or_insert_with(|| imp.name.clone());
                }
            }
            m
        })
        .collect();

    // ---- classes ----------------------------------------------------------------------------
    let mut class_modules: HashMap<String, Vec<(usize, bool)>> = HashMap::new(); // class → (module, is_controller)
    for (i, md) in modules.iter().enumerate() {
        for c in &md.provider_classes {
            class_modules.entry(c.clone()).or_default().push((i, false));
        }
        for c in &md.info.controllers {
            class_modules.entry(c.clone()).or_default().push((i, true));
        }
    }

    let mut diagnostics: Vec<Diagnostic> = vec![];
    let mut classes: Vec<ClassInfo> = vec![];
    let mut routes: Vec<RouteInfo> = vec![];
    let mut missing_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut missing_external_edges: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut inject_edges: Vec<(String, String, bool)> = vec![];

    let (root_modules, global_prefix, port) = find_bootstrap(files);

    for (fi, f) in files.iter().enumerate() {
        if is_spec(&f.rel) {
            continue;
        }
        let p = &f.parsed;
        for c in &p.classes {
            let kind = classify(c, &f.rel);
            let regs = class_modules.get(&c.name).cloned().unwrap_or_default();
            let mut injects = vec![];
            for param in &c.ctor_params {
                let (token, token_kind, forward, optional) = injection_token(p, param);
                injects.push(Injection {
                    param: param.name.clone(),
                    token,
                    token_kind,
                    type_text: param.type_text.clone(),
                    line: param.line,
                    optional,
                    forward_ref: forward,
                    status: InjectionStatus::Unchecked,
                    via: None,
                });
            }

            // Check each injection against every module that registers this class.
            for (reg_idx, (mi, _is_ctrl)) in regs.iter().enumerate() {
                let md = &modules[*mi];
                for inj in injects.iter_mut() {
                    let check = check_injection(&ctx, fi, inj, &available[*mi], &global_tokens, &global_token_source, &providers_of);
                    match check {
                        Check::Resolved(via) => {
                            if reg_idx == 0 {
                                inj.status = InjectionStatus::Resolved;
                                inj.via = Some(via);
                            }
                        }
                        Check::Unchecked => {}
                        Check::Missing(reason) => {
                            if reg_idx == 0 {
                                inj.status = InjectionStatus::Missing;
                            }
                            if inj.optional || md.info.dynamic {
                                continue;
                            }
                            let (detail, fixes) = missing_fix(&modules, &providers_of, &exporters_of, *mi, inj, &reason, &mut missing_edges, &mut missing_external_edges);
                            let subject = format!("{} → {}", c.name, display_token(&inj.token));
                            diagnostics.push(Diagnostic {
                                id: format!("missing:{}:{}:{}", md.info.name, c.name, inj.token),
                                severity: Severity::Error,
                                code: DiagnosticCode::MissingProvider,
                                title: "Missing provider".into(),
                                subject,
                                detail,
                                file: Some(f.rel.clone()),
                                line: inj.line,
                                modules: vec![md.info.name.clone()],
                                fixes,
                            });
                        }
                    }
                }
            }
            for inj in &injects {
                if inj.token_kind == TokenKind::Class && ctx.resolve_class(fi, &inj.token).is_some() {
                    inject_edges.push((c.name.clone(), inj.token.clone(), inj.forward_ref));
                }
            }

            // Unregistered controllers and services.
            if regs.is_empty() && matches!(kind, ClassKind::Controller | ClassKind::Service) {
                let nearest = nearest_module(&modules, &f.rel);
                let (code, title, severity) = if kind == ClassKind::Controller {
                    (DiagnosticCode::ControllerNotRegistered, "Controller not registered", Severity::Warning)
                } else {
                    (DiagnosticCode::ProviderNotRegistered, "Provider not registered", Severity::Info)
                };
                let used_somewhere = files.iter().any(|g| g.parsed.classes.iter().any(|k| k.ctor_params.iter().any(|pp| pp.type_name.as_deref() == Some(&c.name))));
                if kind == ClassKind::Service && used_somewhere {
                    // Reported as a missing provider at the injection site instead.
                } else {
                    let mut fixes = vec![];
                    if let Some(m) = &nearest {
                        let action = if kind == ClassKind::Controller {
                            FixAction::AddController { controller: c.name.clone(), module: m.clone() }
                        } else {
                            FixAction::AddProvider { provider: c.name.clone(), module: m.clone() }
                        };
                        fixes.push(Fix { label: format!("Add to {m}"), action });
                    }
                    diagnostics.push(Diagnostic {
                        id: format!("unregistered:{}", c.name),
                        severity,
                        code,
                        title: title.into(),
                        subject: c.name.clone(),
                        detail: match &nearest {
                            Some(m) => format!("{} isn't listed in any module, so Nest never instantiates it. The closest module is {m}.", c.name),
                            None => format!("{} isn't listed in any module, so Nest never instantiates it.", c.name),
                        },
                        file: Some(f.rel.clone()),
                        line: c.line,
                        modules: nearest.into_iter().collect(),
                        fixes,
                    });
                }
            }

            // Routes.
            let mut prefix = None;
            let mut transport = None;
            if let Some(d) = decorator(c, "Controller") {
                prefix = Some(controller_prefix(p, d));
                transport = Some("rest".to_string());
                if c.methods.iter().any(|m| m.decorators.iter().any(|d| d.name == "MessagePattern" || d.name == "EventPattern")) {
                    transport = Some("microservice".into());
                }
            } else if has_decorator(c, "Resolver") {
                transport = Some("graphql".into());
            } else if has_decorator(c, "WebSocketGateway") {
                transport = Some("ws".into());
            }
            if transport.as_deref() == Some("rest") {
                for m in &c.methods {
                    for d in &m.decorators {
                        let method = match d.name.as_str() {
                            "Get" | "Post" | "Put" | "Patch" | "Delete" | "Options" | "Head" | "All" | "Search" => d.name.to_uppercase(),
                            _ => continue,
                        };
                        let sub = p.decorator_string_arg(d).unwrap_or_default();
                        routes.push(RouteInfo {
                            method,
                            path: join_route(global_prefix.as_deref(), prefix.as_deref().unwrap_or(""), &sub),
                            controller: c.name.clone(),
                            handler: m.name.clone(),
                            file: f.rel.clone(),
                            line: d.line,
                        });
                    }
                }
            }

            let fields = c
                .properties
                .iter()
                .filter(|pr| !pr.name.starts_with('#'))
                .map(|pr| FieldInfo {
                    name: pr.name.clone(),
                    type_text: pr.type_text.clone(),
                    optional: pr.optional,
                    decorators: pr.decorators.iter().map(|d| d.name.clone()).collect(),
                    line: pr.line,
                })
                .collect();
            classes.push(ClassInfo {
                name: c.name.clone(),
                file: f.rel.clone(),
                line: c.line,
                kind,
                exported: c.exported,
                decorators: c.decorators.iter().map(|d| d.name.clone()).collect(),
                injects,
                prefix,
                transport,
                modules: regs.iter().map(|(m, _)| modules[*m].info.name.clone()).collect(),
                extends: c.extends.clone(),
                fields,
            });
        }
    }

    // ---- cycles -----------------------------------------------------------------------------
    let adjacency: Vec<Vec<usize>> = modules.iter().map(|m| m.local_imports.iter().map(|(t, _)| *t).collect()).collect();
    let sccs = tarjan(&adjacency);
    let mut cycle_edges: HashSet<(usize, usize)> = HashSet::new();
    let mut module_cycle_partner: HashMap<usize, String> = HashMap::new();
    for scc in &sccs {
        let set: HashSet<usize> = scc.iter().copied().collect();
        let self_loop = scc.len() == 1 && adjacency[scc[0]].contains(&scc[0]);
        if scc.len() < 2 && !self_loop {
            continue;
        }
        let mut all_forward = true;
        for &a in scc {
            for (b, fwd) in &modules[a].local_imports {
                if set.contains(b) {
                    cycle_edges.insert((a, *b));
                    if !fwd {
                        all_forward = false;
                    }
                }
            }
        }
        if all_forward {
            for &a in scc {
                for (b, _) in &modules[a].local_imports {
                    cycle_edges.remove(&(a, *b));
                }
            }
            continue;
        }
        let mut names: Vec<String> = scc.iter().map(|&i| modules[i].info.name.clone()).collect();
        names.sort();
        for &i in scc {
            let partner = names.iter().find(|n| **n != modules[i].info.name).cloned().unwrap_or_default();
            module_cycle_partner.insert(i, partner);
        }
        let subject = if names.len() == 2 { format!("{} ⇄ {}", names[0], names[1]) } else { format!("{} → {}", names.join(" → "), names[0]) };
        let first = scc.iter().map(|&i| &modules[i]).min_by_key(|m| m.info.name.clone()).unwrap();
        diagnostics.push(Diagnostic {
            id: format!("cycle:{}", names.join(",")),
            severity: Severity::Warning,
            code: DiagnosticCode::CircularImport,
            title: "Circular dependency".into(),
            subject,
            detail: "Nest can't resolve this at bootstrap. Wrap both sides with forwardRef(), or move the shared provider into its own module.".into(),
            file: Some(first.info.file.clone()),
            line: first.info.line,
            modules: names.clone(),
            fixes: vec![Fix { label: "Apply forwardRef".into(), action: FixAction::ApplyForwardRef { modules: names } }],
        });
    }

    // Circular constructor injection between providers.
    {
        let names: Vec<String> = {
            let mut s: BTreeSet<String> = BTreeSet::new();
            for (a, b, _) in &inject_edges {
                s.insert(a.clone());
                s.insert(b.clone());
            }
            s.into_iter().collect()
        };
        let idx: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let mut adj = vec![vec![]; names.len()];
        let mut fwd: HashMap<(usize, usize), bool> = HashMap::new();
        for (a, b, f) in &inject_edges {
            let (ia, ib) = (idx[a.as_str()], idx[b.as_str()]);
            adj[ia].push(ib);
            fwd.insert((ia, ib), *f);
        }
        for scc in tarjan(&adj) {
            if scc.len() < 2 {
                continue;
            }
            let set: HashSet<usize> = scc.iter().copied().collect();
            let unresolved = scc.iter().any(|&a| adj[a].iter().any(|b| set.contains(b) && !fwd.get(&(a, *b)).copied().unwrap_or(false)));
            if !unresolved {
                continue;
            }
            let mut n: Vec<String> = scc.iter().map(|&i| names[i].clone()).collect();
            n.sort();
            let cls = classes.iter().find(|c| c.name == n[0]);
            diagnostics.push(Diagnostic {
                id: format!("inject-cycle:{}", n.join(",")),
                severity: Severity::Warning,
                code: DiagnosticCode::CircularInjection,
                title: "Circular injection".into(),
                subject: if n.len() == 2 { format!("{} ⇄ {}", n[0], n[1]) } else { n.join(" → ") },
                detail: format!("These providers inject each other. Use @Inject(forwardRef(() => {})) on both constructor parameters, or extract the shared logic.", n[1]),
                file: cls.map(|c| c.file.clone()),
                line: cls.map(|c| c.line).unwrap_or(0),
                modules: vec![],
                fixes: vec![],
            });
        }
    }

    // ---- reachability ------------------------------------------------------------------------
    let root_idx: Vec<usize> = root_modules.iter().filter_map(|r| module_by_name.get(r).and_then(|v| v.first().copied())).collect();
    let mut reachable = vec![false; modules.len()];
    let mut queue: VecDeque<usize> = root_idx.iter().copied().collect();
    for &r in &root_idx {
        reachable[r] = true;
    }
    while let Some(m) = queue.pop_front() {
        for (t, _) in &modules[m].local_imports {
            if !reachable[*t] {
                reachable[*t] = true;
                queue.push_back(*t);
            }
        }
    }
    let mut orphans: HashSet<usize> = HashSet::new();
    if let Some(&root) = root_idx.first() {
        for (i, md) in modules.iter().enumerate() {
            if reachable[i] || md.info.file.starts_with("test/") {
                continue;
            }
            // In a monorepo, only modules inside a root's app are expected to be reachable from it.
            if meta.monorepo && !same_app(&md.info.file, &modules[root].info.file) && !md.info.file.starts_with("libs/") {
                continue;
            }
            if md.info.file.starts_with("libs/") {
                continue;
            }
            orphans.insert(i);
            // A missing-provider fix already wires this module in; don't report it twice.
            if missing_edges.iter().any(|(_, t)| *t == i) {
                continue;
            }
            let root_name = modules[root].info.name.clone();
            diagnostics.push(Diagnostic {
                id: format!("orphan:{}", md.info.name),
                severity: Severity::Warning,
                code: DiagnosticCode::ModuleNotImported,
                title: "Module not imported".into(),
                subject: md.info.name.clone(),
                detail: format!("Nothing imports {}, so its controllers and providers never load. Import it into {root_name} or a feature module.", md.info.name),
                file: Some(md.info.file.clone()),
                line: md.info.line,
                modules: vec![md.info.name.clone()],
                fixes: vec![Fix {
                    label: format!("Import into {root_name}"),
                    action: FixAction::ImportModule { module: md.info.name.clone(), into: root_name, package: None },
                }],
            });
        }
    }

    diagnostics.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.id.cmp(&b.id)));
    diagnostics.dedup_by(|a, b| a.id == b.id);

    // ---- graph ---------------------------------------------------------------------------------
    let error_modules: HashSet<String> = diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::MissingProvider)
        .flat_map(|d| d.modules.clone())
        .collect();
    let mut nodes: Vec<GraphNode> = vec![];
    let mut edges: Vec<GraphEdge> = vec![];
    for (i, md) in modules.iter().enumerate() {
        let is_root = root_idx.contains(&i);
        let status = if orphans.contains(&i) {
            NodeStatus::Orphan
        } else if error_modules.contains(&md.info.name) {
            NodeStatus::Error
        } else if module_cycle_partner.contains_key(&i) {
            NodeStatus::Warning
        } else {
            NodeStatus::Ok
        };
        let sublabel = if is_root {
            "root".to_string()
        } else if let Some(p) = module_cycle_partner.get(&i) {
            format!("cycle with {}", p.trim_end_matches("Module"))
        } else if orphans.contains(&i) {
            "not imported".into()
        } else if md.info.global {
            "global".into()
        } else if let Some(orm) = &md.info.orm {
            orm.clone()
        } else if let Some(e) = md.info.exports.first() {
            if md.info.exports.len() == 1 { format!("exports {e}") } else { format!("exports {}", md.info.exports.len()) }
        } else {
            let n = md.provided.len();
            format!("{n} provider{}", if n == 1 { "" } else { "s" })
        };
        nodes.push(GraphNode {
            id: md.info.name.clone(),
            label: md.info.name.clone(),
            sublabel,
            kind: NodeKind::Module,
            file: Some(md.info.file.clone()),
            line: md.info.line,
            x: 0.0,
            y: 0.0,
            w: 176.0,
            h: 64.0,
            status,
            root: is_root,
            global: md.info.global,
            group: None,
        });
        for (t, fwd) in &md.local_imports {
            let kind = if cycle_edges.contains(&(i, *t)) { EdgeKind::Cycle } else { EdgeKind::Import };
            edges.push(GraphEdge { from: md.info.name.clone(), to: modules[*t].info.name.clone(), kind, forward_ref: *fwd, label: None });
        }
    }
    let mut externals: BTreeMap<String, (Option<String>, bool)> = BTreeMap::new();
    for md in &modules {
        for imp in md.info.imports.iter().filter(|i| i.external) {
            let e = externals.entry(imp.name.clone()).or_insert((imp.package.clone(), false));
            e.1 |= imp.global;
            let label = imp.method.clone();
            if !edges.iter().any(|x| x.from == md.info.name && x.to == imp.name) {
                edges.push(GraphEdge { from: md.info.name.clone(), to: imp.name.clone(), kind: EdgeKind::Import, forward_ref: imp.forward_ref, label });
            }
        }
    }
    for (name, (pkg, global)) in &externals {
        nodes.push(GraphNode {
            id: name.clone(),
            label: name.clone(),
            sublabel: if *global { "global".into() } else { pkg.clone().unwrap_or_else(|| "library".into()) },
            kind: NodeKind::External,
            file: None,
            line: 0,
            x: 0.0,
            y: 0.0,
            w: 176.0,
            h: 64.0,
            status: NodeStatus::Ok,
            root: false,
            global: *global,
            group: pkg.clone(),
        });
    }
    for (from, to) in &missing_edges {
        edges.push(GraphEdge { from: modules[*from].info.name.clone(), to: modules[*to].info.name.clone(), kind: EdgeKind::Missing, forward_ref: false, label: None });
    }
    for (from, to) in &missing_external_edges {
        if !nodes.iter().any(|n| &n.id == to) {
            nodes.push(GraphNode {
                id: to.clone(),
                label: to.clone(),
                sublabel: "not imported".into(),
                kind: NodeKind::External,
                file: None,
                line: 0,
                x: 0.0,
                y: 0.0,
                w: 176.0,
                h: 64.0,
                status: NodeStatus::Orphan,
                root: false,
                global: false,
                group: None,
            });
        }
        edges.push(GraphEdge { from: modules[*from].info.name.clone(), to: to.clone(), kind: EdgeKind::Missing, forward_ref: false, label: None });
    }
    let roots_for_layout: Vec<String> = root_idx.iter().map(|&i| modules[i].info.name.clone()).collect();
    let mut graph = Graph { nodes, edges, width: 0.0, height: 0.0 };
    layered_layout(&mut graph, &roots_for_layout);

    // Provider graph.
    let mut pnodes: Vec<GraphNode> = vec![];
    let mut seen_nodes: HashSet<String> = HashSet::new();
    for c in &classes {
        let registered = !c.modules.is_empty();
        let injected = inject_edges.iter().any(|(_, b, _)| b == &c.name);
        if !(registered || injected) || c.kind == ClassKind::Module {
            continue;
        }
        if !matches!(c.kind, ClassKind::Controller | ClassKind::Service | ClassKind::Provider | ClassKind::Gateway | ClassKind::Resolver | ClassKind::Guard | ClassKind::Strategy | ClassKind::Interceptor | ClassKind::Pipe) {
            continue;
        }
        if !seen_nodes.insert(c.name.clone()) {
            continue;
        }
        let missing = c.injects.iter().any(|i| i.status == InjectionStatus::Missing);
        pnodes.push(GraphNode {
            id: c.name.clone(),
            label: c.name.clone(),
            sublabel: c.modules.first().cloned().unwrap_or_else(|| "not registered".into()),
            kind: if c.kind == ClassKind::Controller { NodeKind::Controller } else { NodeKind::Provider },
            file: Some(c.file.clone()),
            line: c.line,
            x: 0.0,
            y: 0.0,
            w: 176.0,
            h: 56.0,
            status: if missing { NodeStatus::Error } else if c.modules.is_empty() { NodeStatus::Orphan } else { NodeStatus::Ok },
            root: c.kind == ClassKind::Controller,
            global: false,
            group: c.modules.first().cloned(),
        });
    }
    let pedges: Vec<GraphEdge> = inject_edges
        .iter()
        .filter(|(a, b, _)| seen_nodes.contains(a) && seen_nodes.contains(b))
        .map(|(a, b, f)| GraphEdge { from: a.clone(), to: b.clone(), kind: EdgeKind::Inject, forward_ref: *f, label: None })
        .collect();
    let proots: Vec<String> = pnodes.iter().filter(|n| n.root).map(|n| n.id.clone()).collect();
    let mut provider_graph = Graph { nodes: pnodes, edges: pedges, width: 0.0, height: 0.0 };
    layered_layout(&mut provider_graph, &proots);

    // ---- files ---------------------------------------------------------------------------------
    let module_dirs: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = modules.iter().map(|m| (parent_dir(&m.info.file).to_string(), m.info.name.clone())).collect();
        v.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        v
    };
    let source_files = files
        .iter()
        .map(|f| {
            let dir = parent_dir(&f.rel);
            let module = module_dirs
                .iter()
                .find(|(d, _)| dir == d || dir.starts_with(&format!("{d}/")))
                .map(|(_, n)| n.clone());
            SourceFileInfo { path: f.rel.clone(), kind: FileKind::from_path(&f.rel), module }
        })
        .collect();

    ProjectModel {
        root: meta.root.to_string_lossy().replace('\\', "/"),
        name: meta.name.clone(),
        nest_version: meta.major_version("@nestjs/core"),
        typescript_version: meta.major_version("typescript"),
        package_manager: meta.package_manager,
        monorepo: meta.monorepo,
        esm: meta.esm,
        source_root: meta.source_root.clone(),
        scripts: meta.scripts.clone(),
        dependencies: meta.deps.keys().cloned().collect(),
        modules: modules.into_iter().map(|m| m.info).collect(),
        classes,
        routes,
        root_modules,
        global_prefix,
        port,
        files: source_files,
        graph,
        provider_graph,
        diagnostics,
        scan_ms: started.elapsed().as_secs_f64() * 1000.0,
        git_branch: crate::project::git_branch(&meta.root),
    }
}

fn same_app(a: &str, b: &str) -> bool {
    let app = |p: &str| p.split('/').take(2).collect::<Vec<_>>().join("/");
    app(a) == app(b)
}

fn collect_exports(modules: &[ModuleData], by_name: &HashMap<String, Vec<usize>>, i: usize, out: &mut BTreeSet<String>, seen: &mut HashSet<usize>) {
    if !seen.insert(i) {
        return;
    }
    let md = &modules[i];
    for e in &md.exports {
        // Re-exporting an imported module passes its exports through.
        if let Some(&(target, _)) = md.local_imports.iter().find(|(t, _)| &modules[*t].info.name == e) {
            collect_exports(modules, by_name, target, out, seen);
            continue;
        }
        if let Some(imp) = md.info.imports.iter().find(|imp| imp.external && &imp.name == e) {
            let (toks, _) = external_tokens(&imp.name, imp.method.as_deref(), &imp.features, false);
            out.extend(toks);
            continue;
        }
        if by_name.contains_key(e) && !md.provided.contains(e) {
            if let Some(&t) = by_name[e].first() {
                collect_exports(modules, by_name, t, out, seen);
            }
            continue;
        }
        out.insert(e.clone());
    }
}

enum Check {
    Resolved(String),
    Missing(MissingReason),
    Unchecked,
}

enum MissingReason {
    LocalClass,
    Feature { orm_module: &'static str, entity: String },
    PackageClass { module: &'static str, package: &'static str },
    Token,
}

fn check_injection(
    ctx: &Ctx,
    file: usize,
    inj: &Injection,
    available: &HashMap<String, String>,
    global: &BTreeSet<String>,
    global_source: &HashMap<String, String>,
    providers_of: &HashMap<String, Vec<usize>>,
) -> Check {
    if let Some(via) = available.get(&inj.token) {
        return Check::Resolved(via.clone());
    }
    if global.contains(&inj.token) {
        let via = global_source.get(&inj.token).cloned().unwrap_or_else(|| "built-in".into());
        return Check::Resolved(via);
    }
    match inj.token_kind {
        TokenKind::Class => {
            if ctx.resolve_class(file, &inj.token).is_some() {
                return Check::Missing(MissingReason::LocalClass);
            }
            if let Some(pkg) = ctx.package_of(file, &inj.token) {
                if let Some((_, module, package)) = PACKAGE_CLASSES.iter().find(|(c, _, p)| *c == inj.token && *p == pkg) {
                    return Check::Missing(MissingReason::PackageClass { module, package });
                }
            }
            Check::Unchecked
        }
        TokenKind::Repository => {
            let entity = inj.token.trim_start_matches("Repository<").trim_end_matches('>').to_string();
            let orm_module = if ctx.meta.has_dep("@mikro-orm/nestjs") && !ctx.meta.has_dep("@nestjs/typeorm") { "MikroOrmModule" } else { "TypeOrmModule" };
            Check::Missing(MissingReason::Feature { orm_module, entity })
        }
        TokenKind::Model => {
            let entity = inj.token.trim_start_matches("Model<").trim_end_matches('>').to_string();
            let orm_module = if ctx.meta.has_dep("@nestjs/sequelize") && !ctx.meta.has_dep("@nestjs/mongoose") { "SequelizeModule" } else { "MongooseModule" };
            Check::Missing(MissingReason::Feature { orm_module, entity })
        }
        TokenKind::Token => {
            if providers_of.contains_key(&inj.token) {
                Check::Missing(MissingReason::Token)
            } else {
                Check::Unchecked
            }
        }
        TokenKind::Unknown => Check::Unchecked,
    }
}

#[allow(clippy::too_many_arguments)]
fn missing_fix(
    modules: &[ModuleData],
    providers_of: &HashMap<String, Vec<usize>>,
    exporters_of: &HashMap<String, Vec<usize>>,
    mi: usize,
    inj: &Injection,
    reason: &MissingReason,
    missing_edges: &mut BTreeSet<(usize, usize)>,
    missing_external: &mut BTreeSet<(usize, String)>,
) -> (String, Vec<Fix>) {
    let m = &modules[mi].info.name;
    let t = display_token(&inj.token);
    match reason {
        MissingReason::Feature { orm_module, entity } => (
            format!("{m} doesn't register {entity} with {orm_module}.forFeature(), so its repository can't be injected."),
            vec![Fix {
                label: format!("Add {entity} to forFeature"),
                action: FixAction::AddFeature { orm_module: orm_module.to_string(), entity: entity.clone(), module: m.clone() },
            }],
        ),
        MissingReason::PackageClass { module, package } => {
            missing_external.insert((mi, module.to_string()));
            (
                format!("{t} comes from {module} ({package}), which {m} doesn't import."),
                vec![Fix {
                    label: format!("Import {module}"),
                    action: FixAction::ImportModule { module: module.to_string(), into: m.clone(), package: Some(package.to_string()) },
                }],
            )
        }
        MissingReason::LocalClass | MissingReason::Token => {
            if let Some(exporters) = exporters_of.get(&inj.token) {
                if let Some(&x) = exporters.iter().find(|&&x| x != mi) {
                    missing_edges.insert((mi, x));
                    let xm = &modules[x].info.name;
                    return (
                        format!("{t} is injected but {m} doesn't import {xm}, which exports it."),
                        vec![Fix { label: format!("Import {xm}"), action: FixAction::ImportModule { module: xm.clone(), into: m.clone(), package: None } }],
                    );
                }
            }
            if let Some(provs) = providers_of.get(&inj.token) {
                if let Some(&x) = provs.iter().find(|&&x| x != mi) {
                    missing_edges.insert((mi, x));
                    let xm = &modules[x].info.name;
                    return (
                        format!("{t} is provided by {xm}, but {xm} doesn't export it and {m} doesn't import it."),
                        vec![Fix {
                            label: format!("Export from {xm} and import"),
                            action: FixAction::ExportAndImport { provider: inj.token.clone(), from: xm.clone(), into: m.clone() },
                        }],
                    );
                }
            }
            if matches!(reason, MissingReason::Token) {
                return (format!("{t} is injected but no module that {m} can see provides it."), vec![]);
            }
            (
                format!("{t} is injected but no module provides it."),
                vec![Fix { label: format!("Add to {m} providers"), action: FixAction::AddProvider { provider: inj.token.clone(), module: m.clone() } }],
            )
        }
    }
}

fn display_token(t: &str) -> String {
    t.to_string()
}

fn nearest_module(modules: &[ModuleData], rel: &str) -> Option<String> {
    let mut dir = parent_dir(rel).to_string();
    loop {
        if let Some(m) = modules.iter().find(|m| parent_dir(&m.info.file) == dir) {
            return Some(m.info.name.clone());
        }
        if dir.is_empty() {
            return None;
        }
        dir = parent_dir(&dir).to_string();
    }
}

fn controller_prefix(p: &Parsed, d: &Decorator) -> String {
    let Some((s, e)) = p.decorator_arg(d, 0) else { return String::new() };
    if let Some(v) = p.string_value(s) {
        if e == s + 1 {
            return v;
        }
    }
    if p.is(s, "[") {
        if let Some(v) = p.string_value(s + 1) {
            return v;
        }
    }
    if p.is(s, "{") {
        for prop in p.object_props(s) {
            if prop.key == "path" {
                if let Some(v) = p.string_value(prop.value.0) {
                    return v;
                }
            }
        }
    }
    String::new()
}

pub fn join_route(global: Option<&str>, prefix: &str, sub: &str) -> String {
    let parts: Vec<&str> = [global.unwrap_or(""), prefix, sub]
        .iter()
        .flat_map(|s| s.split('/'))
        .filter(|s| !s.is_empty())
        .collect();
    format!("/{}", parts.join("/"))
}

/// Finds `NestFactory.create(Root)`, `setGlobalPrefix('x')` and `listen(port)` in main files.
fn find_bootstrap(files: &[FileEntry]) -> (Vec<String>, Option<String>, u32) {
    let mut roots = vec![];
    let mut prefix = None;
    let mut port = 3000;
    for f in files {
        if !f.rel.ends_with("main.ts") {
            continue;
        }
        let p = &f.parsed;
        for i in 0..p.toks.len() {
            if p.is(i, "NestFactory") && p.is(i + 1, ".") && p.is(i + 3, "(") && p.text(i + 2).starts_with("create") && p.kind(i + 4) == Some(Kind::Ident) {
                let name = p.text(i + 4).to_string();
                if !roots.contains(&name) {
                    roots.push(name);
                }
            }
            if p.is(i, "setGlobalPrefix") && p.is(i + 1, "(") {
                prefix = p.string_value(i + 2);
            }
            if p.is(i, "listen") && p.is(i + 1, "(") {
                if let Some(close) = p.matching(i + 1) {
                    for k in i + 2..close {
                        if p.kind(k) == Some(Kind::Num) {
                            if let Ok(n) = p.text(k).parse() {
                                port = n;
                            }
                        }
                    }
                }
            }
        }
    }
    if roots.is_empty() && files.iter().any(|f| f.parsed.classes.iter().any(|c| c.name == "AppModule")) {
        roots.push("AppModule".into());
    }
    (roots, prefix, port)
}

/// Tarjan's strongly connected components.
pub fn tarjan(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct S<'a> {
        adj: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(s: &mut S, v: usize) {
        s.index[v] = Some(s.next);
        s.low[v] = s.next;
        s.next += 1;
        s.stack.push(v);
        s.on[v] = true;
        for &w in &s.adj[v].clone() {
            match s.index[w] {
                None => {
                    visit(s, w);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(iw) if s.on[w] => s.low[v] = s.low[v].min(iw),
                _ => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            let mut comp = vec![];
            while let Some(w) = s.stack.pop() {
                s.on[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            s.out.push(comp);
        }
    }
    let n = adj.len();
    let mut s = S { adj, index: vec![None; n], low: vec![0; n], on: vec![false; n], stack: vec![], next: 0, out: vec![] };
    for v in 0..n {
        if s.index[v].is_none() {
            visit(&mut s, v);
        }
    }
    s.out
}
