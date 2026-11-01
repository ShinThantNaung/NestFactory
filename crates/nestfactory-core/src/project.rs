//! Project metadata: `package.json`, `nest-cli.json`, `tsconfig.json` paths, package
//! manager, git branch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::PackageManager;

#[derive(Debug, Clone)]
pub struct ProjectMeta {
    pub root: PathBuf,
    pub name: String,
    pub deps: BTreeMap<String, String>,
    pub scripts: Vec<String>,
    pub esm: bool,
    pub package_manager: PackageManager,
    pub source_root: String,
    pub monorepo: bool,
    /// tsconfig `paths`: pattern (may end in `*`) → targets.
    pub path_aliases: Vec<(String, Vec<String>)>,
    pub spec_default: bool,
    pub flat_default: bool,
    pub spec_suffix: String,
}

impl ProjectMeta {
    pub fn has_dep(&self, name: &str) -> bool {
        self.deps.contains_key(name)
    }

    pub fn major_version(&self, dep: &str) -> Option<String> {
        let v = self.deps.get(dep)?;
        let digits: String = v
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        (!digits.is_empty()).then_some(digits)
    }
}

pub fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_jsonc(&text)).ok()
}

pub fn read_meta(root: &Path) -> ProjectMeta {
    let pkg = read_json(&root.join("package.json"));
    let mut deps = BTreeMap::new();
    let mut scripts = vec![];
    let mut name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut esm = false;
    if let Some(pkg) = &pkg {
        if let Some(n) = pkg.get("name").and_then(Value::as_str) {
            name = n.to_string();
        }
        esm = pkg.get("type").and_then(Value::as_str) == Some("module");
        for key in ["dependencies", "devDependencies"] {
            if let Some(obj) = pkg.get(key).and_then(Value::as_object) {
                for (k, v) in obj {
                    deps.insert(k.clone(), v.as_str().unwrap_or("").to_string());
                }
            }
        }
        if let Some(obj) = pkg.get("scripts").and_then(Value::as_object) {
            scripts = obj.keys().cloned().collect();
        }
    }

    let cli = read_json(&root.join("nest-cli.json"));
    let mut source_root = "src".to_string();
    let mut monorepo = false;
    let mut spec_default = true;
    let mut flat_default = false;
    let mut spec_suffix = "spec".to_string();
    if let Some(cli) = &cli {
        if let Some(sr) = cli.get("sourceRoot").and_then(Value::as_str) {
            source_root = sr.trim_end_matches('/').to_string();
        }
        monorepo = cli.get("monorepo").and_then(Value::as_bool).unwrap_or(false);
        if let Some(go) = cli.get("generateOptions") {
            if let Some(spec) = go.get("spec").and_then(Value::as_bool) {
                spec_default = spec;
            }
            if let Some(flat) = go.get("flat").and_then(Value::as_bool) {
                flat_default = flat;
            }
            if let Some(sfx) = go.get("specFileSuffix").and_then(Value::as_str) {
                spec_suffix = sfx.to_string();
            }
        }
    }

    let mut path_aliases = vec![];
    if let Some(ts) = read_json(&root.join("tsconfig.json")) {
        let base = ts
            .pointer("/compilerOptions/baseUrl")
            .and_then(Value::as_str)
            .unwrap_or("./")
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_string();
        if let Some(paths) = ts.pointer("/compilerOptions/paths").and_then(Value::as_object) {
            for (pattern, targets) in paths {
                let targets = targets
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(|t| {
                                let t = t.trim_start_matches("./");
                                if base.is_empty() { t.to_string() } else { format!("{base}/{t}") }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                path_aliases.push((pattern.clone(), targets));
            }
        }
    }

    ProjectMeta {
        root: root.to_path_buf(),
        name,
        deps,
        scripts,
        esm,
        package_manager: detect_package_manager(root),
        source_root,
        monorepo,
        path_aliases,
        spec_default,
        flat_default,
        spec_suffix,
    }
}

pub fn detect_package_manager(root: &Path) -> PackageManager {
    if root.join("pnpm-lock.yaml").exists() {
        PackageManager::Pnpm
    } else if root.join("yarn.lock").exists() {
        PackageManager::Yarn
    } else if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        PackageManager::Bun
    } else {
        PackageManager::Npm
    }
}

pub fn git_branch(root: &Path) -> Option<String> {
    let head = std::fs::read_to_string(root.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: refs/heads/") {
        Some(b) => Some(b.to_string()),
        None => Some(head.chars().take(7).collect()),
    }
}

/// Removes `//` and `/* */` comments and trailing commas so tsconfig-style JSON parses.
pub fn strip_jsonc(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    let mut start = 0;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                out.push_str(&s[start..i]);
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                start = i;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                out.push_str(&s[start..i]);
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                start = i;
            }
            b',' => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'}' || b[j] == b']') {
                    out.push_str(&s[start..i]);
                    start = i + 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out.push_str(&s[start..]);
    out
}

/// Normalizes `a/b/../c` style relative paths with forward slashes.
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = vec![];
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

pub fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

/// Relative import specifier from the file `from` to the file `to` (both project-relative),
/// without the `.ts` extension, the way the Nest schematics compute it.
pub fn relative_import(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = parent_dir(from).split('/').filter(|s| !s.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let (to_dir, to_file) = to_parts.split_at(to_parts.len().saturating_sub(1));
    let common = from_dir.iter().zip(to_dir.iter()).take_while(|(a, b)| a == b).count();
    let mut segs: Vec<String> = vec![];
    for _ in common..from_dir.len() {
        segs.push("..".into());
    }
    for s in &to_dir[common..] {
        segs.push((*s).to_string());
    }
    let file = to_file.first().map(|f| f.trim_end_matches(".ts")).unwrap_or("");
    segs.push(file.to_string());
    let joined = segs.join("/");
    if joined.starts_with("..") { joined } else { format!("./{joined}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc() {
        let s = "{ // c\n \"a\": \"//x\", /* y */ \"b\": [1,2,], }";
        let v: Value = serde_json::from_str(&strip_jsonc(s)).unwrap();
        assert_eq!(v["a"], "//x");
        assert_eq!(v["b"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn relative_imports() {
        assert_eq!(relative_import("src/app.module.ts", "src/users/users.module.ts"), "./users/users.module");
        assert_eq!(relative_import("src/users/users.module.ts", "src/users/users.service.ts"), "./users.service");
        assert_eq!(relative_import("src/a/b/x.module.ts", "src/c/y.module.ts"), "../../c/y.module");
    }
}
