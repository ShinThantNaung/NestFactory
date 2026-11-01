//! The npm registry side of the package manager: search, latest versions, and plans that pick
//! the release line fitting the project's Nest before anything is installed. The registry is
//! read from `registry=` in the project's or the user's .npmrc.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use nestfactory_core::model::PackageManager;
use nestfactory_core::pm::{self, InstalledPackage, PackageAction, PackageList, UpdateKind};
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    /// When the latest version was published (ISO 8601).
    pub date: Option<String>,
    pub weekly_downloads: Option<u64>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub npm: String,
    /// The package.json range when the project already depends on it.
    pub installed: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LatestVersion {
    pub name: String,
    /// `None` when the registry couldn't be reached or doesn't have the package.
    pub latest: Option<String>,
    /// From the installed version (or the range, before an install) to `latest`.
    pub update: Option<UpdateKind>,
    /// Set when `latest` needs a newer Nest than the project's: the newest release that
    /// fits it instead (13.2.1 for `@nestjs/apollo` on Nest 11).
    pub newest_for_nest: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlannedPackage {
    pub name: String,
    /// What goes on the command line: `helmet`, `@nestjs/apollo@^13.2.1`, …
    pub spec: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackagePlan {
    pub action: PackageAction,
    pub dev: bool,
    pub packages: Vec<PlannedPackage>,
    /// Required peers the project lacks. Not in `command`; plan again with them to add them.
    pub peers: Vec<PlannedPackage>,
    pub command: String,
    pub notes: Vec<String>,
}

pub fn registry_url(root: &Path) -> String {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    let files = [Some(root.join(".npmrc")), home.map(|h| Path::new(&h).join(".npmrc"))];
    for text in files.into_iter().flatten().filter_map(|f| std::fs::read_to_string(f).ok()) {
        for line in text.lines() {
            let Some(v) = line.trim().strip_prefix("registry").and_then(|r| r.trim_start().strip_prefix('=')) else { continue };
            let v = v.trim().trim_end_matches('/');
            if v.starts_with("http") {
                return v.to_string();
            }
        }
    }
    DEFAULT_REGISTRY.to_string()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build()
}

/// Scoped names keep their `@` but encode the slash, as npm does.
fn encode(name: &str) -> String {
    name.replace('/', "%2f")
}

fn latest(registry: &str, name: &str) -> Option<String> {
    let tags: Value = agent().get(&format!("{registry}/-/package/{}/dist-tags", encode(name))).call().ok()?.into_json().ok()?;
    tags.get("latest")?.as_str().map(String::from)
}

/// The abbreviated packument: every version with its dependencies and peers, no readmes.
fn packument(registry: &str, name: &str) -> Result<Value> {
    agent()
        .get(&format!("{registry}/{}", encode(name)))
        .set("Accept", "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8")
        .call()
        .with_context(|| format!("Couldn't load {name} from {registry}"))?
        .into_json()
        .with_context(|| format!("{registry} sent an unreadable answer for {name}"))
}

pub fn search(registry: &str, query: &str, list: &PackageList) -> Result<Vec<SearchHit>> {
    let res: Value = agent()
        .get(&format!("{registry}/-/v1/search"))
        .query("text", query)
        .query("size", "25")
        .call()
        .with_context(|| format!("Couldn't search {registry}"))?
        .into_json()?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    Ok(res
        .get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            let p = o.get("package")?;
            let name = s(p, "name")?;
            let links = p.get("links").cloned().unwrap_or(Value::Null);
            Some(SearchHit {
                installed: list.packages.iter().find(|i| i.name == name).map(|i| i.range.clone()),
                version: s(p, "version").unwrap_or_default(),
                description: s(p, "description"),
                keywords: p.get("keywords").and_then(Value::as_array).map(|k| k.iter().filter_map(|k| k.as_str().map(String::from)).collect()).unwrap_or_default(),
                date: s(p, "date"),
                weekly_downloads: o.get("downloads").and_then(|d| d.get("weekly")).and_then(Value::as_u64),
                homepage: s(&links, "homepage"),
                repository: s(&links, "repository"),
                npm: s(&links, "npm").unwrap_or_else(|| format!("https://www.npmjs.com/package/{name}")),
                name,
            })
        })
        .collect())
}

/// Latest versions for every dependency, eight registry calls at a time.
pub fn latest_versions(registry: &str, list: &PackageList) -> Vec<LatestVersion> {
    let nest = list.nest_version.as_deref().and_then(|v| v.parse::<u64>().ok());
    let mut out = vec![];
    for chunk in list.packages.chunks(8) {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunk.iter().map(|p| s.spawn(move || latest_version(registry, p, nest))).collect();
            out.extend(handles.into_iter().zip(chunk).map(|(h, p)| h.join().unwrap_or_else(|_| LatestVersion { name: p.name.clone(), latest: None, update: None, newest_for_nest: None })));
        });
    }
    out
}

fn latest_version(registry: &str, p: &InstalledPackage, nest: Option<u64>) -> LatestVersion {
    let latest = latest(registry, &p.name);
    let update = latest.as_deref().and_then(|l| pm::update_kind(p.installed.as_deref().unwrap_or(&p.range), l));
    // Only Nest packages and integrations tie their majors to Nest's; skipping the rest
    // avoids pulling large version lists such as TypeScript's.
    let nest_tied = p.name.starts_with("@nestjs/") || p.name.contains("nest");
    let newest_for_nest = match (update, nest) {
        (Some(UpdateKind::Major), Some(m)) if nest_tied => packument(registry, &p.name).ok().and_then(|doc| pm::newest_for_nest(&doc, &p.name, m)).filter(|v| Some(v) != latest.as_ref()),
        _ => None,
    };
    LatestVersion { name: p.name.clone(), latest, update, newest_for_nest }
}

pub fn plan(registry: &str, pmgr: PackageManager, list: &PackageList, action: PackageAction, specs: &[String], dev: bool) -> PackagePlan {
    let nest = list.nest_version.as_deref().and_then(|v| v.parse::<u64>().ok());
    let mut packages = vec![];
    let mut peers: Vec<PlannedPackage> = vec![];
    let mut notes = vec![];
    match action {
        PackageAction::Add => {
            let requested: Vec<&str> = specs.iter().map(|s| pm::split_spec(s).0).collect();
            let have = |n: &str| requested.contains(&n) || list.packages.iter().any(|p| p.name == n);
            // Version lists can be large (GraphQL's is megabytes), so fetch them side by side.
            let planned: Vec<_> = std::thread::scope(|s| {
                let handles: Vec<_> = specs
                    .iter()
                    .map(|spec| {
                        let (name, version) = pm::split_spec(spec);
                        (name, s.spawn(move || plan_add(registry, name, version, nest)))
                    })
                    .collect();
                handles.into_iter().map(|(name, h)| h.join().unwrap_or_else(|_| (PlannedPackage { name: name.into(), spec: name.into(), note: None }, None))).collect()
            });
            for (p, manifest) in planned {
                for name in manifest.as_ref().map(|m| pm::missing_peers(m, &have)).unwrap_or_default() {
                    if !peers.iter().any(|x| x.name == name) {
                        let spec = match nest {
                            Some(m) if pm::CORE_FAMILY.contains(&name.as_str()) => format!("{name}@^{m}"),
                            _ => name.clone(),
                        };
                        peers.push(PlannedPackage { note: Some(format!("Needed by {}", p.name)), name, spec });
                    }
                }
                packages.push(p);
            }
            if let Some(p) = list.packages.iter().find(|p| requested.contains(&p.name.as_str()) && p.dev != dev) {
                notes.push(format!("{} is already a {}dependency; this moves it to {}.", p.name, if p.dev { "dev " } else { "" }, if dev { "devDependencies" } else { "dependencies" }));
            }
        }
        PackageAction::Remove => {
            for spec in specs {
                let name = pm::split_spec(spec).0.to_string();
                if list.packages.iter().any(|p| p.name == name && p.essential) {
                    notes.push(format!("{name} is needed by every Nest app; removing it breaks the build."));
                }
                packages.push(PlannedPackage { spec: name.clone(), name, note: None });
            }
        }
        PackageAction::Install => {}
    }
    let specs: Vec<String> = packages.iter().map(|p| p.spec.clone()).collect();
    PackagePlan { command: pm::display(&command_for(pmgr, action, &specs, dev)), action, dev, packages, peers, notes }
}

/// Picks the spec for one package to add, and the manifest of the version it resolves to.
fn plan_add(registry: &str, name: &str, version: Option<&str>, nest: Option<u64>) -> (PlannedPackage, Option<Value>) {
    let core = pm::CORE_FAMILY.contains(&name);
    let doc = match packument(registry, name) {
        Ok(doc) => doc,
        Err(e) => {
            let spec = match (version, nest) {
                (Some(v), _) => format!("{name}@{v}"),
                (None, Some(m)) if core => format!("{name}@^{m}"),
                _ => name.to_string(),
            };
            return (PlannedPackage { name: name.into(), spec, note: Some(format!("{e:#}; the package manager resolves the version.")) }, None);
        }
    };
    let manifest_of = |v: &str| doc.get("versions").and_then(|vs| vs.get(v)).cloned();
    let latest = doc.get("dist-tags").and_then(|t| t.get("latest")).and_then(Value::as_str).map(String::from);
    if let Some(v) = version {
        return (PlannedPackage { name: name.into(), spec: format!("{name}@{v}"), note: None }, manifest_of(v));
    }
    let Some(m) = nest else {
        return (PlannedPackage { name: name.into(), spec: name.into(), note: latest.as_ref().map(|l| format!("Latest: {l}")) }, latest.as_deref().and_then(manifest_of));
    };
    match pm::newest_for_nest(&doc, name, m) {
        Some(pick) if Some(&pick) == latest.as_ref() => (PlannedPackage { name: name.into(), spec: name.into(), note: Some(format!("Latest: {pick}")) }, manifest_of(&pick)),
        Some(pick) => {
            let why = latest.map(|l| format!(" ({l} needs a newer Nest)")).unwrap_or_default();
            let planned = PlannedPackage { name: name.into(), spec: format!("{name}@^{pick}"), note: Some(format!("{pick} is the newest release for Nest {m}{why}")) };
            (planned, manifest_of(&pick))
        }
        None => {
            let planned = PlannedPackage { name: name.into(), spec: name.into(), note: Some(format!("No release of {name} lists Nest {m} as supported; expect peer dependency errors.")) };
            (planned, latest.as_deref().and_then(manifest_of))
        }
    }
}

pub fn command_for(pmgr: PackageManager, action: PackageAction, specs: &[String], dev: bool) -> Vec<String> {
    match action {
        PackageAction::Add => pm::add(pmgr, specs, dev),
        PackageAction::Remove => pm::remove(pmgr, specs),
        PackageAction::Install => pm::install(pmgr),
    }
}
