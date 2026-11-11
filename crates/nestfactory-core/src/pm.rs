//! Package manager commands, the project's packages, and picking versions that fit its Nest.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::model::PackageManager;
use crate::project::{read_json, read_meta};

pub fn program(pm: PackageManager) -> &'static str {
    match pm {
        PackageManager::Npm => "npm",
        PackageManager::Pnpm => "pnpm",
        PackageManager::Yarn => "yarn",
        PackageManager::Bun => "bun",
    }
}

pub fn add(pm: PackageManager, packages: &[String], dev: bool) -> Vec<String> {
    let mut cmd = vec![program(pm).to_string()];
    cmd.push(if pm == PackageManager::Npm { "install" } else { "add" }.to_string());
    if dev {
        cmd.push("-D".into());
    }
    cmd.extend(packages.iter().cloned());
    cmd
}

pub fn install(pm: PackageManager) -> Vec<String> {
    vec![program(pm).to_string(), "install".to_string()]
}

pub fn run_script(pm: PackageManager, script: &str) -> Vec<String> {
    match pm {
        PackageManager::Yarn => vec!["yarn".into(), script.into()],
        _ => vec![program(pm).into(), "run".into(), script.into()],
    }
}

/// `npx`-style execution of a local binary.
pub fn exec(pm: PackageManager, bin: &str, args: &[&str]) -> Vec<String> {
    let mut cmd: Vec<String> = match pm {
        PackageManager::Npm => vec!["npx".into(), "--no-install".into()],
        PackageManager::Pnpm => vec!["pnpm".into(), "exec".into()],
        PackageManager::Yarn => vec!["yarn".into()],
        PackageManager::Bun => vec!["bunx".into()],
    };
    cmd.push(bin.into());
    cmd.extend(args.iter().map(|s| s.to_string()));
    cmd
}

pub fn display(cmd: &[String]) -> String {
    cmd.iter()
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn remove(pm: PackageManager, packages: &[String]) -> Vec<String> {
    let mut cmd = vec![program(pm).to_string()];
    cmd.push(if pm == PackageManager::Npm { "uninstall" } else { "remove" }.to_string());
    cmd.extend(packages.iter().cloned());
    cmd
}

// ---- the project's packages -------------------------------------------------------------------

/// Released in lockstep with `@nestjs/core`; a project keeps them on one major.
pub const CORE_FAMILY: &[&str] = &[
    "@nestjs/common",
    "@nestjs/core",
    "@nestjs/platform-express",
    "@nestjs/platform-fastify",
    "@nestjs/platform-socket.io",
    "@nestjs/platform-ws",
    "@nestjs/microservices",
    "@nestjs/websockets",
    "@nestjs/testing",
];

/// Removing one of these breaks the app or the build.
const ESSENTIAL: &[&str] = &["@nestjs/common", "@nestjs/core", "@nestjs/platform-express", "@nestjs/platform-fastify", "reflect-metadata", "rxjs", "typescript", "@nestjs/cli"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PackageAction {
    Add,
    Remove,
    /// A plain `npm install` from package.json and the lockfile.
    Install,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledPackage {
    pub name: String,
    /// The range in package.json.
    pub range: String,
    /// The version in node_modules; `None` until the package is installed.
    pub installed: Option<String>,
    pub dev: bool,
    pub essential: bool,
    /// Source files that import it; filled by [`usages`].
    pub used_in: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackageList {
    pub package_manager: PackageManager,
    pub nest_version: Option<String>,
    pub packages: Vec<InstalledPackage>,
    /// Something in package.json isn't in node_modules yet.
    pub needs_install: bool,
    /// The lockfile the package manager was picked from, if any.
    pub lockfile: Option<String>,
}

pub fn list(root: &Path) -> PackageList {
    let meta = read_meta(root);
    let pkg = read_json(&root.join("package.json"));
    let mut packages = vec![];
    for (key, dev) in [("dependencies", false), ("devDependencies", true)] {
        let Some(obj) = pkg.as_ref().and_then(|p| p.get(key)).and_then(Value::as_object) else { continue };
        for (name, range) in obj {
            let manifest = read_json(&root.join("node_modules").join(name).join("package.json"));
            packages.push(InstalledPackage {
                name: name.clone(),
                range: range.as_str().unwrap_or("").to_string(),
                installed: manifest.and_then(|m| m.get("version").and_then(Value::as_str).map(String::from)),
                dev,
                essential: ESSENTIAL.contains(&name.as_str()),
                used_in: vec![],
            });
        }
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    PackageList {
        package_manager: meta.package_manager,
        nest_version: meta.major_version("@nestjs/core"),
        needs_install: packages.iter().any(|p| p.installed.is_none()),
        lockfile: ["pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb", "package-lock.json"].into_iter().find(|f| root.join(f).is_file()).map(String::from),
        packages,
    }
}

/// Splits `name@range` into its parts; the `@` of a scope isn't a separator.
pub fn split_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.get(1..).and_then(|s| s.find('@')) {
        Some(i) => (&spec[..i + 1], Some(&spec[i + 2..])),
        None => (spec, None),
    }
}

// ---- versions ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateKind {
    Major,
    Minor,
    Patch,
}

/// `1.2.3` (or a range such as `^1.2.3`) → (1, 2, 3); `None` for prereleases and tags.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(|c: char| !c.is_ascii_digit());
    if v.contains('-') {
        return None;
    }
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

pub fn update_kind(from: &str, to: &str) -> Option<UpdateKind> {
    let (a, b) = (parse_version(from)?, parse_version(to)?);
    if b <= a {
        None
    } else if b.0 != a.0 {
        Some(UpdateKind::Major)
    } else if b.1 != a.1 {
        Some(UpdateKind::Minor)
    } else {
        Some(UpdateKind::Patch)
    }
}

/// Whether a peer range such as `^10.0.0 || ^11.0.0` or `>=9 <12` admits a major version.
/// Only majors matter here: they decide which release line fits the project's Nest.
pub fn range_allows_major(range: &str, major: u64) -> bool {
    range.split("||").any(|alt| {
        let alt = alt.trim();
        if alt.is_empty() || alt == "*" || alt == "x" || alt == "latest" {
            return true;
        }
        alt.split_whitespace().all(|c| {
            let Some(i) = c.find(|ch: char| ch.is_ascii_digit()) else { return true };
            let (op, rest) = c.split_at(i);
            let Some(m) = rest.split('.').next().and_then(|m| m.parse::<u64>().ok()) else { return true };
            // `<12.1` still admits 12.0 and `>12.0.0` admits 12.0.1; `<12`, `<12.0.0` and `>12` don't.
            let past_major_start = rest.split('.').skip(1).any(|p| p.parse::<u64>().is_ok_and(|n| n > 0));
            match op {
                ">=" => major >= m,
                ">" => major > m || (major == m && rest.contains('.')),
                "<" => major < m || (major == m && past_major_start),
                "<=" => major <= m,
                _ => major == m,
            }
        })
    })
}

/// The newest stable version in an (abbreviated) npm packument that fits a project on Nest
/// `nest_major`: the same major for the core family, otherwise `@nestjs/common` and
/// `@nestjs/core` peer ranges that admit it. Packages without a Nest peer fit any project.
pub fn newest_for_nest(packument: &Value, name: &str, nest_major: u64) -> Option<String> {
    let versions = packument.get("versions")?.as_object()?;
    let mut best: Option<((u64, u64, u64), &str)> = None;
    for (v, manifest) in versions {
        let Some(parsed) = parse_version(v) else { continue };
        let fits = if CORE_FAMILY.contains(&name) {
            parsed.0 == nest_major
        } else {
            let peers = manifest.get("peerDependencies");
            ["@nestjs/common", "@nestjs/core"]
                .iter()
                .filter_map(|p| peers.and_then(|ps| ps.get(*p)).and_then(Value::as_str))
                .all(|r| range_allows_major(r, nest_major))
        };
        if fits && best.is_none_or(|(b, _)| parsed > b) {
            best = Some((parsed, v));
        }
    }
    best.map(|(_, v)| v.to_string())
}

/// The peers a version needs that the project doesn't have, skipping optional ones.
pub fn missing_peers(manifest: &Value, have: &dyn Fn(&str) -> bool) -> Vec<String> {
    let optional = |n: &str| manifest.get("peerDependenciesMeta").and_then(|m| m.get(n)).and_then(|m| m.get("optional")).and_then(Value::as_bool).unwrap_or(false);
    manifest
        .get("peerDependencies")
        .and_then(Value::as_object)
        .map(|ps| ps.keys().filter(|n| !optional(n) && !have(n)).cloned().collect())
        .unwrap_or_default()
}

// ---- catalog ----------------------------------------------------------------------------------

/// A Nest package worth offering by name, with what it needs alongside it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogEntry {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    /// Installed together with `name`.
    pub companions: Vec<String>,
    pub dev_companions: Vec<String>,
    /// Already in package.json.
    pub installed: bool,
    pub docs: String,
    /// Published under `@nestjs`.
    pub official: bool,
    /// How to set it up when NestFactory has no wiring for it.
    pub hint: Option<String>,
}

struct Row {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    category: &'static str,
    companions: &'static [&'static str],
    dev: &'static [&'static str],
    docs: &'static str,
    hint: &'static str,
}

const fn row(name: &'static str, title: &'static str, description: &'static str, category: &'static str, companions: &'static [&'static str], docs: &'static str) -> Row {
    Row { name, title, description, category, companions, dev: &[], docs, hint: "" }
}

const NEST_DOCS: &str = "https://docs.nestjs.com/";

/// Official packages first, then third-party ones, each roughly by how often Nest apps use them.
const CATALOG: &[Row] = &[
    row("@nestjs/config", "Config", ".env loading and validated config through ConfigService", "Validation", &[], "techniques/configuration"),
    row("@nestjs/swagger", "Swagger", "OpenAPI docs from decorators", "Transport & realtime", &[], "openapi/introduction"),
    Row { hint: "Set up the connection in the Database view; it installs a driver and writes the module config.", ..row("@nestjs/typeorm", "TypeORM", "TypeORM integration for SQL databases", "Data & ORM", &["typeorm"], "techniques/database") },
    Row { hint: "Set up the connection in the Database view.", ..row("@nestjs/mongoose", "Mongoose", "MongoDB schemas and models", "Data & ORM", &["mongoose"], "techniques/mongodb") },
    row("@nestjs/terminus", "Health checks", "Readiness and liveness checks for databases, memory and disk", "Observability", &[], "recipes/terminus"),
    row("@nestjs/schedule", "Scheduling", "Cron jobs, intervals and timeouts", "Queues & jobs", &[], "techniques/task-scheduling"),
    row("@nestjs/throttler", "Rate limiting", "Rate limiting guard", "Security & auth", &[], "security/rate-limiting"),
    row("@nestjs/jwt", "JWT", "Sign and verify JWTs", "Security & auth", &[], "security/authentication"),
    Row { dev: &["@types/passport-jwt"], ..row("@nestjs/passport", "Passport", "Authentication strategies (local, JWT, OAuth) as guards", "Security & auth", &["passport", "passport-jwt"], "recipes/passport") },
    row("@nestjs/bullmq", "Queues", "Redis-backed job queues", "Queues & jobs", &["bullmq"], "techniques/queues"),
    row("@nestjs/event-emitter", "Events", "In-process events with @OnEvent", "Queues & jobs", &[], "techniques/events"),
    row("@nestjs/cqrs", "CQRS", "Commands, queries, events and sagas", "Queues & jobs", &[], "recipes/cqrs"),
    row("@nestjs/cache-manager", "Caching", "In-memory or Redis cache with an interceptor", "Data & ORM", &["cache-manager"], "techniques/caching"),
    row("@nestjs/graphql", "GraphQL", "Code-first or schema-first GraphQL with Apollo", "Transport & realtime", &["@nestjs/apollo", "@apollo/server", "@as-integrations/express5", "graphql"], "graphql/quick-start"),
    Row { hint: "Generate a gateway from the Generate view.", ..row("@nestjs/websockets", "WebSockets", "Gateways over Socket.IO", "Transport & realtime", &["@nestjs/platform-socket.io"], "websockets/gateways") },
    Row { hint: "Generate a resource with the Microservice transport from the Generate view.", ..row("@nestjs/microservices", "Microservices", "TCP, Redis, NATS, Kafka, RabbitMQ and gRPC transports", "Transport & realtime", &[], "microservices/basics") },
    row("@nestjs/axios", "HTTP client", "Axios as an injectable HttpService", "Transport & realtime", &["axios"], "techniques/http-module"),
    Row { hint: "Add ServeStaticModule.forRoot({ rootPath }) to AppModule imports, pointing at your built client.", ..row("@nestjs/serve-static", "Static files", "Serve a single-page app or a public folder", "Transport & realtime", &[], "recipes/serve-static") },
    row("class-validator", "Validation", "DTO validation rules", "Validation", &["class-transformer"], "https://docs.nestjs.com/techniques/validation"),
    Row { hint: "Pass a Joi schema as validationSchema to ConfigModule.forRoot().", ..row("joi", "Joi", "Schema validation, e.g. for environment variables", "Validation", &[], "https://docs.nestjs.com/techniques/configuration#schema-validation") },
    Row { hint: "Call app.use(helmet()) in main.ts before app.listen().", ..row("helmet", "Helmet", "Security headers middleware", "Security & auth", &[], "https://docs.nestjs.com/security/helmet") },
    Row { dev: &["@types/bcrypt"], ..row("bcrypt", "bcrypt", "Password hashing · native build", "Security & auth", &[], "https://github.com/kelektiv/node.bcrypt.js") },
    row("nestjs-pino", "Pino logger", "Fast JSON request logging", "Observability", &["pino-http"], "https://github.com/iamolegga/nestjs-pino"),
    Row { hint: "Import an instrument.ts that calls Sentry.init() first thing in main.ts, then add SentryModule.forRoot().", ..row("@sentry/nestjs", "Sentry", "Error and performance monitoring", "Observability", &[], "https://docs.sentry.io/platforms/javascript/guides/nestjs/") },
];

pub fn catalog(have: &dyn Fn(&str) -> bool) -> Vec<CatalogEntry> {
    CATALOG.iter().map(|r| catalog_entry(r, have(r.name))).collect()
}

pub fn catalog_lookup(name: &str, installed: bool) -> Option<CatalogEntry> {
    CATALOG.iter().find(|r| r.name == name).map(|r| catalog_entry(r, installed))
}

fn catalog_entry(r: &Row, installed: bool) -> CatalogEntry {
    let strings = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
    CatalogEntry {
        name: r.name.into(),
        title: r.title.into(),
        description: r.description.into(),
        category: r.category.into(),
        companions: strings(r.companions),
        dev_companions: strings(r.dev),
        installed,
        docs: if r.docs.starts_with("http") { r.docs.into() } else { format!("{NEST_DOCS}{}", r.docs) },
        official: r.name.starts_with("@nestjs/"),
        hint: (!r.hint.is_empty()).then(|| r.hint.into()),
    }
}

/// Which source files import each package, by package name.
pub fn usages(root: &Path, files: &[String]) -> std::collections::BTreeMap<String, Vec<String>> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\(\s*)['"]([^'"./][^'"]*)['"]"#).unwrap());
    let mut out: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for f in files {
        let Ok(text) = std::fs::read_to_string(root.join(f)) else { continue };
        let mut seen = std::collections::BTreeSet::new();
        for c in re.captures_iter(&text) {
            let spec = &c[1];
            let mut parts = spec.splitn(3, '/');
            let first = parts.next().unwrap_or_default();
            let name = if first.starts_with('@') { format!("{first}/{}", parts.next().unwrap_or_default()) } else { first.trim_start_matches("node:").to_string() };
            if seen.insert(name.clone()) {
                out.entry(name).or_default().push(f.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn specs() {
        assert_eq!(split_spec("@nestjs/config"), ("@nestjs/config", None));
        assert_eq!(split_spec("@nestjs/config@^4"), ("@nestjs/config", Some("^4")));
        assert_eq!(split_spec("helmet@8.1.0"), ("helmet", Some("8.1.0")));
        assert_eq!(remove(PackageManager::Npm, &["helmet".into()]), ["npm", "uninstall", "helmet"]);
        assert_eq!(remove(PackageManager::Pnpm, &["helmet".into()]), ["pnpm", "remove", "helmet"]);
    }

    #[test]
    fn versions() {
        assert_eq!(update_kind("11.0.1", "12.0.0"), Some(UpdateKind::Major));
        assert_eq!(update_kind("^4.0.0", "4.1.0"), Some(UpdateKind::Minor));
        assert_eq!(update_kind("4.1.0", "4.1.2"), Some(UpdateKind::Patch));
        assert_eq!(update_kind("4.1.2", "4.1.2"), None);
        assert_eq!(update_kind("4.1.2", "5.0.0-beta.1"), None);
        assert!(range_allows_major("^10.0.0 || ^11.0.0", 11));
        assert!(!range_allows_major("^10.0.0 || ^11.0.0", 12));
        assert!(range_allows_major(">=9 <12", 11));
        assert!(!range_allows_major(">=9 <12", 12));
        assert!(!range_allows_major(">=9 <12.0.0", 12));
        assert!(range_allows_major(">=9 <12.1", 12));
        assert!(range_allows_major(">12.0.0", 12));
        assert!(!range_allows_major(">12", 12));
        assert!(range_allows_major("11.x", 11));
        assert!(range_allows_major("*", 3));
    }

    #[test]
    fn picks_the_release_line_for_the_projects_nest() {
        let packument = json!({ "versions": {
            "13.2.0": { "peerDependencies": { "@nestjs/common": "^11.0.0", "@nestjs/core": "^11.0.0" } },
            "13.2.1": { "peerDependencies": { "@nestjs/common": "^11.0.0", "@nestjs/core": "^11.0.0" } },
            "14.0.0-next.1": { "peerDependencies": { "@nestjs/common": "^11.0.0 || ^12.0.0" } },
            "14.1.1": { "peerDependencies": { "@nestjs/common": "^12.0.0", "@nestjs/core": "^12.0.0" } }
        }});
        assert_eq!(newest_for_nest(&packument, "@nestjs/apollo", 11).as_deref(), Some("13.2.1"));
        assert_eq!(newest_for_nest(&packument, "@nestjs/apollo", 12).as_deref(), Some("14.1.1"));
        assert_eq!(newest_for_nest(&packument, "@nestjs/apollo", 9), None);
        let core = json!({ "versions": { "11.1.6": {}, "12.0.1": {}, "12.1.0-next.0": {} } });
        assert_eq!(newest_for_nest(&core, "@nestjs/websockets", 11).as_deref(), Some("11.1.6"));
        let plain = json!({ "versions": { "8.0.0": {}, "8.1.0": {} } });
        assert_eq!(newest_for_nest(&plain, "helmet", 11).as_deref(), Some("8.1.0"));
    }

    #[test]
    fn finds_which_files_import_a_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.ts"), "import { Cron } from '@nestjs/schedule';\nimport 'reflect-metadata';\nimport { x } from './x';\nconst b = require('bcrypt');\nimport { join } from 'node:path';\nimport { Y } from 'lodash/fp';\n").unwrap();
        std::fs::write(dir.path().join("src/b.ts"), "import { Interval } from \"@nestjs/schedule/dist/x\";\n").unwrap();
        let u = usages(dir.path(), &["src/a.ts".into(), "src/b.ts".into()]);
        assert_eq!(u["@nestjs/schedule"], ["src/a.ts", "src/b.ts"]);
        assert_eq!(u["bcrypt"], ["src/a.ts"]);
        assert_eq!(u["lodash"], ["src/a.ts"]);
        assert!(u.contains_key("reflect-metadata") && !u.contains_key("./x") && !u.contains_key("x"));
    }

    #[test]
    fn peers() {
        let m = json!({
            "peerDependencies": { "@nestjs/common": "^12", "graphql": "^16", "@apollo/server": "^5", "@as-integrations/fastify": "^3" },
            "peerDependenciesMeta": { "@as-integrations/fastify": { "optional": true } }
        });
        let mut got = missing_peers(&m, &|n| n == "@nestjs/common");
        got.sort();
        assert_eq!(got, ["@apollo/server", "graphql"]);
    }
}
