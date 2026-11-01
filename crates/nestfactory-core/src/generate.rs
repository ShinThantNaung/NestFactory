//! Visual generators. Renders the official Nest schematic templates with the same options,
//! naming and module wiring as `nest g`, into a [`Changeset`] that is previewed first.
//! Team templates in `.nestfactory/templates/<schematic>/` override the built-in ones.

use std::path::Path;

use anyhow::{bail, Result};
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::changeset::{Changeset, Tree};
use crate::edit;
use crate::naming::{capitalize, classify, normalize_name};
use crate::project::{normalize_path, ProjectMeta};
use crate::template::{render, render_path, Value, Vars};

static NEST_TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates/nest");

pub const TEAM_TEMPLATE_DIR: &str = ".nestfactory/templates";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Schematic {
    Resource,
    Module,
    Controller,
    Service,
    Provider,
    Guard,
    Pipe,
    Interceptor,
    Filter,
    Middleware,
    Gateway,
    Resolver,
    Decorator,
    Class,
    Interface,
    Dto,
}

impl Schematic {
    pub fn id(self) -> &'static str {
        match self {
            Schematic::Resource => "resource",
            Schematic::Module => "module",
            Schematic::Controller => "controller",
            Schematic::Service => "service",
            Schematic::Provider => "provider",
            Schematic::Guard => "guard",
            Schematic::Pipe => "pipe",
            Schematic::Interceptor => "interceptor",
            Schematic::Filter => "filter",
            Schematic::Middleware => "middleware",
            Schematic::Gateway => "gateway",
            Schematic::Resolver => "resolver",
            Schematic::Decorator => "decorator",
            Schematic::Class => "class",
            Schematic::Interface => "interface",
            Schematic::Dto => "dto",
        }
    }

    /// Module metadata key and class suffix used when wiring into the nearest module.
    fn wiring(self) -> Option<(&'static str, Option<&'static str>)> {
        match self {
            Schematic::Resource | Schematic::Module => Some(("imports", Some("module"))),
            Schematic::Controller => Some(("controllers", Some("controller"))),
            Schematic::Service => Some(("providers", Some("service"))),
            Schematic::Gateway => Some(("providers", Some("gateway"))),
            Schematic::Resolver => Some(("providers", Some("resolver"))),
            Schematic::Provider => Some(("providers", None)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Transport {
    Rest,
    GraphqlCodeFirst,
    GraphqlSchemaFirst,
    Microservice,
    Ws,
}

impl Transport {
    pub fn id(self) -> &'static str {
        match self {
            Transport::Rest => "rest",
            Transport::GraphqlCodeFirst => "graphql-code-first",
            Transport::GraphqlSchemaFirst => "graphql-schema-first",
            Transport::Microservice => "microservice",
            Transport::Ws => "ws",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DtoField {
    pub name: String,
    /// string | number | int | boolean | date | email | uuid | url | enum
    pub ty: String,
    pub optional: bool,
    pub array: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GenerateOptions {
    pub schematic: Schematic,
    pub name: String,
    /// Project-relative directory to generate in, usually the target module's folder.
    /// Defaults to the source root.
    pub directory: Option<String>,
    pub transport: Transport,
    pub crud: bool,
    pub spec: bool,
    pub flat: bool,
    /// Use Swagger's `PartialType` and `@ApiProperty` on DTOs.
    pub swagger: bool,
    pub skip_import: bool,
    pub fields: Vec<DtoField>,
}

/// Template files for a schematic: team templates first, then the vendored Nest ones.
fn template_files(root: &Path, schematic: &str) -> (Vec<(String, String)>, bool) {
    let team = root.join(TEAM_TEMPLATE_DIR).join(schematic);
    if team.is_dir() {
        let mut out = vec![];
        for e in ignore::WalkBuilder::new(&team).hidden(false).build().flatten() {
            if e.file_type().is_some_and(|t| t.is_file()) {
                if let (Ok(rel), Ok(text)) = (e.path().strip_prefix(&team), std::fs::read_to_string(e.path())) {
                    out.push((rel.to_string_lossy().replace('\\', "/"), text));
                }
            }
        }
        if !out.is_empty() {
            out.sort();
            return (out, true);
        }
    }
    let mut out = vec![];
    if let Some(dir) = NEST_TEMPLATES.get_dir(schematic) {
        collect_embedded(dir, schematic, &mut out);
    }
    out.sort();
    (out, false)
}

fn collect_embedded(dir: &Dir, prefix: &str, out: &mut Vec<(String, String)>) {
    for f in dir.files() {
        let rel = f.path().to_string_lossy().replace('\\', "/");
        let rel = rel.strip_prefix(&format!("{prefix}/")).unwrap_or(&rel).to_string();
        if let Some(text) = f.contents_utf8() {
            out.push((rel, text.replace("\r\n", "\n")));
        }
    }
    for d in dir.dirs() {
        collect_embedded(d, prefix, out);
    }
}

/// Finds the nearest `*.module.ts` from `dir` upward, like the schematics' `ModuleFinder`.
pub fn find_module(tree: &Tree, dir: &str) -> Option<String> {
    let mut cur = dir.to_string();
    loop {
        if let Some(f) = tree.list_files(&cur).into_iter().find(|f| f.ends_with(".module.ts") || f.ends_with(".module.js")) {
            return Some(if cur.is_empty() { f } else { format!("{cur}/{f}") });
        }
        if cur.is_empty() {
            return None;
        }
        cur = crate::project::parent_dir(&cur).to_string();
    }
}

/// `PathSolver.relative`, for the import path written into the module.
fn solver_relative(from: &str, to: &str) -> String {
    let rel = crate::project::relative_import(from, &format!("{to}.ts"));
    rel
}

pub fn generate(root: &Path, meta: &ProjectMeta, opts: &GenerateOptions) -> Result<Changeset> {
    let raw = opts.name.trim().replace('\\', "/");
    if raw.is_empty() {
        bail!("Enter a name");
    }
    if raw.chars().any(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | ' '))) {
        bail!("Names can only contain letters, digits, '-', '_', '.' and '/'");
    }
    let (name_dir, base) = match raw.rfind('/') {
        Some(i) => (raw[..i].to_string(), raw[i + 1..].to_string()),
        None => (String::new(), raw.clone()),
    };
    let mut name = normalize_name(&base);
    if opts.schematic == Schematic::Dto {
        name = name.trim_end_matches("-dto").trim_end_matches(".dto").to_string();
    }
    let base_dir = opts.directory.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| meta.source_root.clone());
    let mut path = normalize_path(&format!("{base_dir}/{}", normalize_name(&name_dir)));
    let schematic = opts.schematic;
    if schematic == Schematic::Dto {
        if !opts.flat {
            path = format!("{path}/dto");
        }
    } else if !opts.flat {
        path = normalize_path(&format!("{path}/{name}"));
    }

    let mut tree = Tree::with_esm(root, meta.esm);
    let mut notes = vec![];
    let mut cs_install = vec![];
    let mut run_install = false;
    let type_id = if schematic == Schematic::Resource { opts.transport.id() } else { schematic.wiring().and_then(|w| w.1).unwrap_or("") };
    let spec_suffix = normalize_name(&meta.spec_suffix);

    let class_name = if name.contains('.') { classify(&name).replacen('.', "", 1) } else if schematic == Schematic::Provider { classify(&name) } else { name.clone() };

    // Resource: Swagger's PartialType when available, else @nestjs/mapped-types, as the CLI does.
    let mut swagger_installed = false;
    // (The CLI does this even without CRUD, so NestFactory does too.)
    if schematic == Schematic::Resource && opts.transport != Transport::GraphqlCodeFirst {
        if opts.transport == Transport::Rest && (meta.has_dep("@nestjs/swagger") || opts.swagger) {
            swagger_installed = true;
            if !meta.has_dep("@nestjs/swagger") {
                cs_install.push("@nestjs/swagger".to_string());
                notes.push("Installs @nestjs/swagger for PartialType and API docs.".into());
            }
        } else if !meta.has_dep("@nestjs/mapped-types") {
            if let Some(pkg) = tree.read("package.json") {
                if let Some(next) = edit::add_package_dependency(&pkg, "@nestjs/mapped-types", "*", false) {
                    tree.write("package.json", next);
                    run_install = true;
                    notes.push("Adds @nestjs/mapped-types to package.json, like the Nest CLI.".into());
                }
            }
        }
    }
    if matches!(opts.transport, Transport::GraphqlCodeFirst | Transport::GraphqlSchemaFirst) && schematic == Schematic::Resource && !meta.has_dep("@nestjs/graphql") {
        notes.push("GraphQL resources need @nestjs/graphql and a GraphQLModule in the root module.".into());
    }
    if (opts.transport == Transport::Ws && schematic == Schematic::Resource || schematic == Schematic::Gateway) && !meta.has_dep("@nestjs/websockets") {
        cs_install.push("@nestjs/websockets".into());
        cs_install.push("@nestjs/platform-socket.io".into());
    }
    if opts.transport == Transport::Microservice && schematic == Schematic::Resource && !meta.has_dep("@nestjs/microservices") {
        cs_install.push("@nestjs/microservices".into());
    }

    let mut vars = Vars::new();
    vars.insert("name".into(), Value::from(name.clone()));
    vars.insert("className".into(), Value::from(class_name.clone()));
    vars.insert("type".into(), Value::from(type_id));
    vars.insert("crud".into(), Value::from(opts.crud));
    vars.insert("isEsm".into(), Value::from(meta.esm));
    vars.insert("isSwaggerInstalled".into(), Value::from(swagger_installed));
    vars.insert("specFileSuffix".into(), Value::from(spec_suffix.clone()));
    vars.insert("spec".into(), Value::from(opts.spec));
    vars.insert("flat".into(), Value::from(opts.flat));
    vars.insert("language".into(), Value::from("ts"));
    vars.insert("sourceRoot".into(), Value::from(meta.source_root.clone()));

    // Wire into the nearest module before writing the new files, as the schematics do.
    if let (false, Some((metadata, element_type))) = (opts.skip_import, schematic.wiring()) {
        if let Some(module_path) = find_module(&tree, &path) {
            let symbol = match element_type {
                Some(t) => format!("{}{}", classify(&name), capitalize(t)),
                None => class_name.clone(),
            };
            let target = match element_type {
                Some(t) => format!("{path}/{name}.{t}"),
                None => format!("{path}/{name}"),
            };
            let mut import_path = solver_relative(&module_path, &target);
            if meta.esm {
                import_path.push_str(".js");
            }
            let before = tree.read(&module_path).unwrap_or_default();
            let had_key = crate::ts::parse::Parsed::new(before.as_str()).classes.iter().any(|c| c.decorators.iter().any(|d| d.name == "Module"))
                && before.contains(&format!("{metadata}:"));
            let after = edit::declare(&before, metadata, &symbol, &import_path);
            let module_name = module_path.rsplit('/').next().unwrap_or(&module_path).to_string();
            if after == before {
                notes.push(format!("{symbol} is already wired in {module_name}; it is left unchanged."));
            } else {
                if had_key {
                    notes.push(format!("Existing {metadata} array found — {symbol} is added once, nothing else in the file changes."));
                } else {
                    notes.push(format!("Adds a {metadata} array to {module_name} with {symbol}."));
                }
                tree.write(&module_path, after);
            }
        } else {
            notes.push("No module found above this folder, so nothing is wired. Import it manually.".into());
        }
    }

    // Render files.
    let mut team = false;
    if schematic == Schematic::Dto {
        let (files, is_team) = template_files(root, "dto");
        if is_team {
            team = true;
            render_files(&mut tree, &files, &path, &vars, opts, schematic)?;
        } else {
            let (content, packages) = dto_source(&name, &opts.fields, opts.swagger || meta.has_dep("@nestjs/swagger"));
            tree.write(&format!("{path}/{name}.dto.ts"), content);
            for p in packages {
                if !meta.has_dep(&p) {
                    cs_install.push(p);
                }
            }
        }
    } else {
        let (files, is_team) = template_files(root, schematic.id());
        team = is_team;
        if files.is_empty() {
            bail!("No templates for {}", schematic.id());
        }
        render_files(&mut tree, &files, &path, &vars, opts, schematic)?;
    }
    if team {
        notes.push(format!("Using the team template in {TEAM_TEMPLATE_DIR}/{}.", schematic.id()));
    }

    let rel_label = path.clone();
    notes.insert(0, format!("All files go to {rel_label}/, matching the Nest CLI layout."));

    let command = cli_command(meta, opts, &base_dir, &name_dir, &name);
    let label = format!("Generate {} {}", schematic.id(), name);
    let mut cs = tree.into_changeset(label, command);
    cs.notes = notes;
    cs.install = cs_install;
    cs.run_install = run_install;
    Ok(cs)
}

/// New files follow the project's line endings (the module or main file decides).
fn project_uses_crlf(tree: &Tree, source_root: &str, path: &str) -> bool {
    let probe = find_module(tree, path).or_else(|| Some(format!("{source_root}/main.ts")));
    probe.and_then(|p| tree.read(&p)).is_some_and(|c| c.contains("\r\n"))
}

fn render_files(tree: &mut Tree, files: &[(String, String)], path: &str, vars: &Vars, opts: &GenerateOptions, schematic: Schematic) -> Result<()> {
    let source_root = match vars.get("sourceRoot") {
        Some(Value::Str(s)) => s.clone(),
        _ => "src".to_string(),
    };
    let crlf = project_uses_crlf(tree, &source_root, path);
    let t = opts.transport;
    let graphql = matches!(t, Transport::GraphqlCodeFirst | Transport::GraphqlSchemaFirst);
    for (rel, text) in files {
        if !opts.spec && rel.contains(".__specFileSuffix__.") {
            continue;
        }
        if schematic == Schematic::Resource {
            let keep = if rel.ends_with(".dto.ts") {
                !graphql && opts.crud
            } else if rel.ends_with(".input.ts") {
                graphql && opts.crud
            } else if rel.ends_with(".resolver.ts") || rel.ends_with(".resolver.__specFileSuffix__.ts") {
                graphql
            } else if rel.ends_with(".graphql") {
                t == Transport::GraphqlSchemaFirst && opts.crud
            } else if rel.ends_with("controller.ts") || rel.ends_with(".controller.__specFileSuffix__.ts") {
                matches!(t, Transport::Microservice | Transport::Rest)
            } else if rel.ends_with(".gateway.ts") || rel.ends_with(".gateway.__specFileSuffix__.ts") {
                t == Transport::Ws
            } else if rel.contains("@ent") {
                opts.crud
            } else {
                true
            };
            if !keep {
                continue;
            }
        }
        let out_rel = render_path(rel, vars)?;
        let mut content = render(text, vars).map_err(|e| anyhow::anyhow!("{rel}: {e}"))?;
        if crlf {
            content = content.replace('\n', "\r\n");
        }
        let dest = normalize_path(&format!("{path}/{out_rel}"));
        if tree.exists(&dest) {
            bail!("{dest} already exists");
        }
        tree.write(&dest, content);
    }
    Ok(())
}

fn cli_command(meta: &ProjectMeta, opts: &GenerateOptions, base_dir: &str, name_dir: &str, name: &str) -> Option<String> {
    let mut rel = base_dir.to_string();
    let mut project = None;
    if let Some(r) = rel.strip_prefix(&meta.source_root) {
        rel = r.trim_start_matches('/').to_string();
    } else {
        let parts: Vec<&str> = base_dir.split('/').collect();
        if parts.len() >= 3 && (parts[0] == "apps" || parts[0] == "libs") && parts[2] == "src" {
            project = Some(parts[1].to_string());
            rel = parts[3..].join("/");
        }
    }
    let mut segs: Vec<String> = vec![];
    if !rel.is_empty() {
        segs.push(rel);
    }
    if !name_dir.is_empty() {
        segs.push(normalize_name(name_dir));
    }
    let schematic = if opts.schematic == Schematic::Dto { "class" } else { opts.schematic.id() };
    let target = if opts.schematic == Schematic::Dto { format!("dto/{name}.dto") } else { name.to_string() };
    segs.push(target);
    let mut cmd = format!("nest g {schematic} {}", segs.join("/"));
    if opts.schematic == Schematic::Resource {
        cmd.push_str(&format!(" --type {}", opts.transport.id()));
        cmd.push_str(if opts.crud { " --crud" } else { " --crud=false" });
    }
    if !opts.spec && opts.schematic != Schematic::Dto {
        cmd.push_str(" --no-spec");
    }
    if opts.schematic == Schematic::Dto {
        cmd.push_str(" --flat --no-spec");
    } else if opts.flat {
        cmd.push_str(" --flat");
    }
    if opts.skip_import {
        cmd.push_str(" --skip-import");
    }
    if let Some(p) = project {
        cmd.push_str(&format!(" --project {p}"));
    }
    Some(cmd)
}

/// A DTO class with `class-validator` rules and optional Swagger decorators.
pub fn dto_source(name: &str, fields: &[DtoField], swagger: bool) -> (String, Vec<String>) {
    let class = format!("{}Dto", classify(name));
    let mut validators: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut body = vec![];
    let mut needs_type = false;
    for f in fields {
        let mut lines = vec![];
        let (ts, rule) = match f.ty.as_str() {
            "number" => ("number", "IsNumber"),
            "int" => ("number", "IsInt"),
            "boolean" => ("boolean", "IsBoolean"),
            "date" => ("Date", "IsDate"),
            "email" => ("string", "IsEmail"),
            "uuid" => ("string", "IsUUID"),
            "url" => ("string", "IsUrl"),
            _ => ("string", "IsString"),
        };
        if swagger {
            lines.push(if f.optional { "  @ApiProperty({ required: false })".to_string() } else { "  @ApiProperty()".to_string() });
        }
        if f.optional {
            validators.insert("IsOptional");
            lines.push("  @IsOptional()".into());
        }
        if f.array {
            validators.insert("IsArray");
            lines.push("  @IsArray()".into());
        }
        validators.insert(rule);
        let each = if f.array {
            if matches!(rule, "IsNumber") { "{}, { each: true }" } else { "{ each: true }" }
        } else if rule == "IsNumber" {
            ""
        } else {
            ""
        };
        lines.push(format!("  @{rule}({each})"));
        if ts == "Date" {
            needs_type = true;
            lines.push("  @Type(() => Date)".into());
        }
        let ty = if f.array { format!("{ts}[]") } else { ts.to_string() };
        let field = crate::naming::camelize(&f.name);
        lines.push(format!("  {field}{}: {ty};", if f.optional { "?" } else { "" }));
        body.push(lines.join("\n"));
    }
    let mut imports = vec![];
    let mut packages = vec![];
    if swagger && !fields.is_empty() {
        imports.push("import { ApiProperty } from '@nestjs/swagger';".to_string());
    }
    if needs_type {
        imports.push("import { Type } from 'class-transformer';".to_string());
    }
    if !validators.is_empty() {
        imports.push(format!("import {{ {} }} from 'class-validator';", validators.iter().copied().collect::<Vec<_>>().join(", ")));
        packages.push("class-validator".to_string());
        packages.push("class-transformer".to_string());
    }
    let header = if imports.is_empty() { String::new() } else { format!("{}\n\n", imports.join("\n")) };
    let content = if body.is_empty() {
        format!("{header}export class {class} {{}}\n")
    } else {
        format!("{header}export class {class} {{\n{}\n}}\n", body.join("\n\n"))
    };
    (content, packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_with_rules() {
        let fields = vec![
            DtoField { name: "customer".into(), ty: "string".into(), optional: false, array: false },
            DtoField { name: "amount".into(), ty: "int".into(), optional: true, array: false },
        ];
        let (src, pkgs) = dto_source("create-invoice", &fields, true);
        assert!(src.contains("export class CreateInvoiceDto {"));
        assert!(src.contains("import { IsInt, IsOptional, IsString } from 'class-validator';"));
        assert!(src.contains("  @ApiProperty({ required: false })\n  @IsOptional()\n  @IsInt()\n  amount?: number;"));
        assert_eq!(pkgs, vec!["class-validator", "class-transformer"]);
    }
}
