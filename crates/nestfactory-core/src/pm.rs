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
            });
        }
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    PackageList {
        package_manager: meta.package_manager,
        nest_version: meta.major_version("@nestjs/core"),
        needs_install: packages.iter().any(|p| p.installed.is_none()),
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
}

type CatalogRow = (&'static str, &'static str, &'static str, &'static str, &'static [&'static str], &'static [&'static str], &'static str);

const CATALOG: &[CatalogRow] = &[
    ("@nestjs/config", "Config", "Load .env files and read settings through ConfigService", "Essentials", &[], &[], "techniques/configuration"),
    ("class-validator", "Validation", "Decorator rules on DTOs, checked by ValidationPipe", "Essentials", &["class-transformer"], &[], "techniques/validation"),
    ("@nestjs/swagger", "Swagger", "OpenAPI spec and Swagger UI from controllers and DTOs", "Essentials", &[], &[], "openapi/introduction"),
    ("@nestjs/terminus", "Health checks", "Readiness and liveness endpoints for databases, memory and disk", "Essentials", &[], &[], "recipes/terminus"),
    ("@nestjs/jwt", "JWT", "Sign and verify JSON Web Tokens", "Security", &[], &[], "security/authentication"),
    ("@nestjs/passport", "Passport", "Authentication strategies (local, JWT, OAuth) as guards", "Security", &["passport", "passport-jwt"], &["@types/passport-jwt"], "recipes/passport"),
    ("@nestjs/throttler", "Rate limiting", "Limit requests per client with a global or per-route guard", "Security", &[], &[], "security/rate-limiting"),
    ("helmet", "Helmet", "Secure HTTP response headers", "Security", &[], &[], "security/helmet"),
    ("@nestjs/typeorm", "TypeORM", "SQL entities and repositories (the Database view also picks a driver)", "Data", &["typeorm"], &[], "techniques/database"),
    ("@nestjs/mongoose", "Mongoose", "MongoDB schemas and models", "Data", &["mongoose"], &[], "techniques/mongodb"),
    ("@nestjs/cache-manager", "Caching", "In-memory or Redis cache with an interceptor", "Data", &["cache-manager"], &[], "techniques/caching"),
    ("@nestjs/graphql", "GraphQL", "Code-first or schema-first GraphQL with Apollo", "APIs", &["@nestjs/apollo", "@apollo/server", "@as-integrations/express5", "graphql"], &[], "graphql/quick-start"),
    ("@nestjs/websockets", "WebSockets", "Gateways over Socket.IO", "APIs", &["@nestjs/platform-socket.io"], &[], "websockets/gateways"),
    ("@nestjs/microservices", "Microservices", "TCP, Redis, NATS, Kafka, RabbitMQ and gRPC transports", "APIs", &[], &[], "microservices/basics"),
    ("@nestjs/axios", "HTTP client", "Axios as an injectable HttpService", "APIs", &["axios"], &[], "techniques/http-module"),
    ("@nestjs/serve-static", "Static files", "Serve a single-page app or a public folder", "APIs", &[], &[], "recipes/serve-static"),
    ("@nestjs/schedule", "Scheduling", "Cron jobs, intervals and timeouts", "Background work", &[], &[], "techniques/task-scheduling"),
    ("@nestjs/bullmq", "Queues", "Redis-backed job queues with BullMQ", "Background work", &["bullmq"], &[], "techniques/queues"),
    ("@nestjs/event-emitter", "Events", "Decoupled in-process events with @OnEvent", "Background work", &[], &[], "techniques/events"),
    ("@nestjs/cqrs", "CQRS", "Commands, queries, events and sagas", "Architecture", &[], &[], "recipes/cqrs"),
];

pub fn catalog(have: &dyn Fn(&str) -> bool) -> Vec<CatalogEntry> {
    CATALOG
        .iter()
        .map(|(name, title, description, category, companions, dev_companions, docs)| CatalogEntry {
            name: name.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            category: category.to_string(),
            companions: companions.iter().map(|s| s.to_string()).collect(),
            dev_companions: dev_companions.iter().map(|s| s.to_string()).collect(),
            installed: have(name),
            docs: format!("https://docs.nestjs.com/{docs}"),
        })
        .collect()
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
