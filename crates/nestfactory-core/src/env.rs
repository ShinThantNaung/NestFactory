//! Environment and config manager: `.env` files, the Config module schema (class-validator
//! `EnvironmentVariables` or a Joi `validationSchema`), usage in code, and the issues
//! between them: missing, unused, empty and invalid variables.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::scanner::collect_ts_files;
use crate::ts::parse::Parsed;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvEntry {
    pub key: String,
    pub value: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvFile {
    pub path: String,
    pub example: bool,
    pub entries: Vec<EnvEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvSchemaVar {
    pub key: String,
    pub required: bool,
    pub ty: String,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvUsage {
    pub key: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EnvIssueKind {
    Missing,
    Unused,
    Empty,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvIssue {
    pub file: String,
    pub key: String,
    pub kind: EnvIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvReport {
    pub files: Vec<EnvFile>,
    pub schema: Vec<EnvSchemaVar>,
    pub schema_source: Option<String>,
    pub usages: Vec<EnvUsage>,
    pub issues: Vec<EnvIssue>,
}

fn key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_.]*)\s*=\s*(.*)$").unwrap())
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    for q in ['"', '\'', '`'] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return v[1..v.len() - 1].to_string();
        }
    }
    // Strip trailing inline comments on unquoted values.
    match v.find(" #") {
        Some(i) => v[..i].trim_end().to_string(),
        None => v.to_string(),
    }
}

pub fn parse_env(content: &str) -> Vec<EnvEntry> {
    let mut out = vec![];
    for (i, line) in content.lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(c) = key_re().captures(line) {
            out.push(EnvEntry { key: c[1].to_string(), value: unquote(&c[2]), line: i as u32 + 1 });
        }
    }
    out
}

fn quote_if_needed(v: &str) -> String {
    if v.is_empty() || v.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@%+,=?&".contains(c)) {
        v.to_string()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Sets (`Some`) or removes (`None`) keys, keeping comments, order and blank lines.
pub fn update_env(content: &str, updates: &[(String, Option<String>)]) -> String {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut i = 0;
    while i < lines.len() {
        let key = key_re().captures(&lines[i]).filter(|_| !lines[i].trim_start().starts_with('#')).map(|c| c[1].to_string());
        if let Some(k) = key {
            if let Some((_, v)) = updates.iter().find(|(uk, _)| *uk == k) {
                match v {
                    Some(v) => {
                        lines[i] = format!("{k}={}", quote_if_needed(v));
                        done.insert(k);
                    }
                    None => {
                        lines.remove(i);
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    let new: Vec<&(String, Option<String>)> = updates.iter().filter(|(k, v)| v.is_some() && !done.contains(k)).collect();
    if !new.is_empty() && lines.last().is_some_and(|l| !l.trim().is_empty()) {
        lines.push(String::new());
    }
    for (k, v) in new {
        lines.push(format!("{k}={}", quote_if_needed(v.as_deref().unwrap_or(""))));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    if content.contains("\r\n") {
        out = out.replace('\n', "\r\n");
    }
    out
}

/// Adds a property to the class-validator `EnvironmentVariables` class if missing.
pub fn add_schema_property(src: &str, key: &str, ty: &str) -> String {
    let p = Parsed::new(src);
    let Some(class) = p.classes.iter().find(|c| c.name == "EnvironmentVariables") else { return src.to_string() };
    if class.properties.iter().any(|pr| pr.name == key) {
        return src.to_string();
    }
    let close = p.toks[class.body.1].start;
    let decorator = if ty == "number" { "IsNumber" } else { "IsString" };
    let mut out = src[..close].trim_end().to_string();
    out.push_str(&format!("\n\n  @{decorator}()\n  {key}: {ty};\n"));
    out.push_str(&src[close..]);
    crate::edit::insert_import(&out, decorator, "class-validator")
}

fn schema_from_class(p: &Parsed) -> Option<Vec<EnvSchemaVar>> {
    let class = p.classes.iter().find(|c| c.name == "EnvironmentVariables" || (c.properties.iter().filter(|pr| is_env_key(&pr.name)).count() >= 2 && c.properties.iter().any(|pr| pr.decorators.iter().any(|d| d.name.starts_with("Is")))))?;
    let vars = class
        .properties
        .iter()
        .filter(|pr| is_env_key(&pr.name))
        .map(|pr| {
            let decs: Vec<&str> = pr.decorators.iter().map(|d| d.name.as_str()).collect();
            let ty = if decs.iter().any(|d| matches!(*d, "IsNumber" | "IsInt" | "IsPort" | "Min" | "Max")) {
                "number"
            } else if decs.iter().any(|d| matches!(*d, "IsBoolean" | "IsBooleanString")) {
                "boolean"
            } else if decs.contains(&"IsEnum") {
                "enum"
            } else if decs.iter().any(|d| matches!(*d, "IsUrl" | "IsUri")) {
                "url"
            } else {
                "string"
            };
            EnvSchemaVar {
                key: pr.name.clone(),
                required: !pr.optional && !pr.has_initializer && !decs.contains(&"IsOptional"),
                ty: ty.into(),
                default: pr.initializer.clone(),
            }
        })
        .collect();
    Some(vars)
}

fn schema_from_joi(p: &Parsed) -> Option<Vec<EnvSchemaVar>> {
    for i in 0..p.toks.len() {
        if p.is(i, "Joi") && p.is(i + 1, ".") && p.is(i + 2, "object") && p.is(i + 3, "(") && p.is(i + 4, "{") {
            let mut out = vec![];
            for prop in p.object_props(i + 4) {
                if !is_env_key(&prop.key) {
                    continue;
                }
                let text = p.slice(prop.value.0, prop.value.1);
                let ty = ["number", "boolean", "string"].iter().find(|t| text.contains(&format!("Joi.{t}("))).copied().unwrap_or(if text.contains(".valid(") { "enum" } else { "string" });
                static DEF: OnceLock<Regex> = OnceLock::new();
                let def = DEF.get_or_init(|| Regex::new(r"\.default\(([^)]*)\)").unwrap());
                out.push(EnvSchemaVar {
                    key: prop.key.clone(),
                    required: text.contains(".required()"),
                    ty: ty.into(),
                    default: def.captures(text).map(|c| c[1].trim().to_string()),
                });
            }
            return Some(out);
        }
    }
    None
}

fn is_env_key(k: &str) -> bool {
    !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && k.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn usage_res() -> &'static [Regex; 3] {
    static RE: OnceLock<[Regex; 3]> = OnceLock::new();
    RE.get_or_init(|| {
        [
            Regex::new(r"process\.env\.([A-Z_][A-Z0-9_]*)").unwrap(),
            Regex::new(r#"process\.env\[\s*['"]([A-Z_][A-Z0-9_]*)['"]\s*\]"#).unwrap(),
            Regex::new(r#"\.(?:get|getOrThrow)\s*(?:<[^>()]*>)?\(\s*['"]([A-Z][A-Z0-9_]*)['"]"#).unwrap(),
        ]
    })
}

pub fn env_files(root: &Path) -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.file_type().is_ok_and(|t| t.is_file()) && (name == ".env" || name.starts_with(".env.")) {
                out.push(name);
            }
        }
    }
    out.sort_by_key(|n| (n != ".env", n.clone()));
    out
}

pub fn report(root: &Path) -> EnvReport {
    let mut files = vec![];
    for name in env_files(root) {
        let content = std::fs::read_to_string(root.join(&name)).unwrap_or_default();
        let example = name.contains("example") || name.contains("sample") || name.contains("template");
        files.push(EnvFile { path: name, example, entries: parse_env(&content) });
    }
    let mut schema = vec![];
    let mut schema_source = None;
    let mut usages = vec![];
    for (abs, rel) in collect_ts_files(root) {
        if rel.ends_with(".spec.ts") || rel.ends_with(".e2e-spec.ts") {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&abs) else { continue };
        if schema_source.is_none() && (src.contains("EnvironmentVariables") || src.contains("Joi.object")) {
            let p = Parsed::new(src.as_str());
            if let Some(s) = schema_from_class(&p).or_else(|| schema_from_joi(&p)) {
                if !s.is_empty() {
                    schema = s;
                    schema_source = Some(rel.clone());
                }
            }
        }
        for (i, line) in src.lines().enumerate() {
            for re in usage_res() {
                for c in re.captures_iter(line) {
                    usages.push(EnvUsage { key: c[1].to_string(), file: rel.clone(), line: i as u32 + 1 });
                }
            }
        }
    }
    let used: BTreeSet<&str> = usages.iter().map(|u| u.key.as_str()).collect();
    let schema_keys: BTreeMap<&str, &EnvSchemaVar> = schema.iter().map(|s| (s.key.as_str(), s)).collect();
    let mut issues = vec![];
    let main = files.iter().find(|f| f.path == ".env").or_else(|| files.iter().find(|f| !f.example));
    for f in files.iter().filter(|f| !f.example) {
        let keys: BTreeMap<&str, &EnvEntry> = f.entries.iter().map(|e| (e.key.as_str(), e)).collect();
        for s in &schema {
            match keys.get(s.key.as_str()) {
                None if s.required && f.path == ".env" => issues.push(EnvIssue {
                    file: f.path.clone(),
                    key: s.key.clone(),
                    kind: EnvIssueKind::Missing,
                    message: format!("{} is required by the Config schema but not set.", s.key),
                }),
                Some(e) if s.required && e.value.is_empty() => issues.push(EnvIssue {
                    file: f.path.clone(),
                    key: s.key.clone(),
                    kind: EnvIssueKind::Empty,
                    message: format!("{} is required but empty.", s.key),
                }),
                Some(e) if !e.value.is_empty() && !valid_for(&s.ty, &e.value) => issues.push(EnvIssue {
                    file: f.path.clone(),
                    key: s.key.clone(),
                    kind: EnvIssueKind::Invalid,
                    message: format!("{} should be a {}, got \"{}\".", s.key, s.ty, e.value),
                }),
                _ => {}
            }
        }
        if f.path == ".env" {
            for k in &used {
                if !keys.contains_key(k) && !schema_keys.get(k).is_some_and(|s| s.default.is_some() || !s.required) && !matches!(*k, "NODE_ENV" | "PORT") {
                    let first = usages.iter().find(|u| u.key == *k).unwrap();
                    issues.push(EnvIssue {
                        file: f.path.clone(),
                        key: k.to_string(),
                        kind: EnvIssueKind::Missing,
                        message: format!("{k} is read in {}:{} but not set.", first.file, first.line),
                    });
                }
            }
        }
        for e in &f.entries {
            if !used.contains(e.key.as_str()) && !schema_keys.contains_key(e.key.as_str()) && !is_tooling_key(&e.key) {
                issues.push(EnvIssue {
                    file: f.path.clone(),
                    key: e.key.clone(),
                    kind: EnvIssueKind::Unused,
                    message: format!("{} isn't read anywhere in the code or the schema.", e.key),
                });
            }
        }
    }
    if let (Some(main), Some(example)) = (main, files.iter().find(|f| f.example)) {
        for e in &main.entries {
            if !example.entries.iter().any(|x| x.key == e.key) {
                issues.push(EnvIssue {
                    file: example.path.clone(),
                    key: e.key.clone(),
                    kind: EnvIssueKind::Missing,
                    message: format!("{} is in {} but not documented in {}.", e.key, main.path, example.path),
                });
            }
        }
    }
    issues.dedup_by(|a, b| a.file == b.file && a.key == b.key && a.kind == b.kind);
    EnvReport { files, schema, schema_source, usages, issues }
}

/// Keys read by tools rather than app code (Prisma, Docker Compose, Node).
fn is_tooling_key(k: &str) -> bool {
    matches!(k, "DATABASE_URL" | "NODE_ENV" | "PORT" | "TZ" | "NODE_OPTIONS" | "DEBUG" | "COMPOSE_PROJECT_NAME") || k.starts_with("DB_")
}

fn valid_for(ty: &str, v: &str) -> bool {
    match ty {
        "number" => v.parse::<f64>().is_ok(),
        "boolean" => matches!(v, "true" | "false" | "1" | "0"),
        "url" => v.contains("://"),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_updates() {
        let src = "# db\nDB_HOST=localhost\nexport DB_PORT=5432 # comment\nNAME=\"a b\"\n";
        let e = parse_env(src);
        assert_eq!(e.len(), 3);
        assert_eq!(e[1].value, "5432");
        assert_eq!(e[2].value, "a b");
        let out = update_env(src, &[("DB_PORT".into(), Some("5433".into())), ("NAME".into(), None), ("NEW".into(), Some("x y".into()))]);
        assert_eq!(out, "# db\nDB_HOST=localhost\nDB_PORT=5433\n\nNEW=\"x y\"\n");
    }

    #[test]
    fn schema_property() {
        let src = "import { IsString } from 'class-validator';\n\nclass EnvironmentVariables {\n  @IsString()\n  JWT_SECRET: string;\n}\n";
        let out = add_schema_property(src, "DB_PORT", "number");
        assert!(out.contains("  @IsNumber()\n  DB_PORT: number;\n}"), "{out}");
        assert!(out.contains("import { IsString, IsNumber } from 'class-validator';"));
        let p = Parsed::new(out.as_str());
        let s = schema_from_class(&p).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].ty, "number");
        assert!(s[1].required);
    }

    #[test]
    fn joi_schema() {
        let src = "ConfigModule.forRoot({ validationSchema: Joi.object({ PORT: Joi.number().default(3000), JWT_SECRET: Joi.string().required() }) })";
        let s = schema_from_joi(&Parsed::new(src)).unwrap();
        assert_eq!(s[0].default.as_deref(), Some("3000"));
        assert!(s[1].required);
    }
}
