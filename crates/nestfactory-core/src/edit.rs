//! Source edits for module wiring. `insert_metadata` and `insert_import` port the Nest
//! schematics' `MetadataManager` and `ModuleImportDeclarator` so the output is byte-for-byte
//! what `nest g` produces, with one addition: nothing is inserted twice.

use std::sync::OnceLock;

use regex::Regex;

use crate::ts::lexer::Kind;
use crate::ts::parse::{Decorator, Parsed};

fn insert_at(content: &str, pos: usize, text: &str) -> String {
    let mut s = String::with_capacity(content.len() + text.len());
    s.push_str(&content[..pos]);
    s.push_str(text);
    s.push_str(&content[pos..]);
    s
}

/// Keeps CRLF files CRLF after inserting LF text.
fn match_eol(original: &str, edited: String) -> String {
    if original.contains("\r\n") {
        edited.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        edited
    }
}

/// The `{` token of the first `@Module({...})`-style decorator (case-insensitive, like Nest).
fn module_object(p: &Parsed, decorator: &str) -> Option<usize> {
    let mut all: Vec<&Decorator> = p.classes.iter().flat_map(|c| c.decorators.iter()).collect();
    all.sort_by_key(|d| d.start);
    for d in all {
        if !d.name.eq_ignore_ascii_case(decorator) {
            continue;
        }
        if let Some((s, _)) = p.decorator_arg(d, 0) {
            if p.is(s, "{") {
                return Some(s);
            }
        }
    }
    None
}

/// Full text of the token range including the leading trivia after the previous token.
fn full_text(p: &Parsed, s: usize, e: usize) -> &str {
    let start = if s > 0 { p.toks[s - 1].end } else { 0 };
    &p.src[start..p.toks[e - 1].end]
}

/// Adds `symbol` to `@Module({ <metadata>: [...] })`. Returns `None` if there is no
/// `@Module` object to edit, and the content unchanged if the symbol is already listed.
pub fn insert_metadata(content: &str, metadata: &str, symbol: &str) -> Option<String> {
    insert_decorator_metadata(content, "Module", metadata, symbol)
}

pub fn insert_decorator_metadata(content: &str, decorator: &str, metadata: &str, symbol: &str) -> Option<String> {
    let p = Parsed::new(content);
    let open = module_object(&p, decorator)?;
    let close = p.matching(open)?;
    let items = p.split_items(open, close);
    let props = p.object_props(open);
    let matching = props.iter().find(|pr| !pr.spread && pr.key == metadata && pr.value.0 != pr.key_tok);
    let out = match matching {
        None if items.is_empty() => {
            let pos = p.toks[close].start;
            insert_at(content, pos, &format!("\n  {metadata}: [{symbol}]\n"))
        }
        None => {
            let (s, e) = *items.last().unwrap();
            let pos = p.toks[e - 1].end;
            static NL: OnceLock<Regex> = OnceLock::new();
            let nl = NL.get_or_init(|| Regex::new(r"^\r?\n\s*").unwrap());
            let text = full_text(&p, s, e);
            let to_insert = match nl.find(text) {
                Some(m) => format!(",{}{metadata}: [{symbol}]", m.as_str()),
                None => format!(", {metadata}: [{symbol}]"),
            };
            insert_at(content, pos, &to_insert)
        }
        Some(prop) => {
            let (vs, ve) = prop.value;
            if !p.is(vs, "[") || p.matching(vs) != Some(ve - 1) {
                return Some(content.to_string());
            }
            let arr_close = ve - 1;
            let elements = p.split_items(vs, arr_close);
            if elements.is_empty() {
                insert_at(content, p.toks[arr_close].start, symbol)
            } else {
                if elements.iter().any(|&(s, e)| p.slice(s, e) == symbol) {
                    return Some(content.to_string());
                }
                let (s, e) = *elements.last().unwrap();
                let text = full_text(&p, s, e);
                static SEP1: OnceLock<Regex> = OnceLock::new();
                static SEP2: OnceLock<Regex> = OnceLock::new();
                let sep1 = SEP1.get_or_init(|| Regex::new(r"^\r?\n(\r?)\s+").unwrap());
                let sep2 = SEP2.get_or_init(|| Regex::new(r"^\r?\n").unwrap());
                let sep = sep1.find(text).or_else(|| sep2.find(text)).map(|m| m.as_str()).unwrap_or(" ");
                insert_at(content, p.toks[e - 1].end, &format!(",{sep}{symbol}"))
            }
        }
    };
    Some(match_eol(content, out))
}

/// Whether `symbol` is already imported (from any path) in this file.
pub fn has_import(content: &str, symbol: &str) -> bool {
    let p = Parsed::new(content);
    p.imports.iter().any(|i| i.names.iter().any(|(_, local)| local == symbol) || i.default.as_deref() == Some(symbol))
}

/// Adds `import { symbol } from 'path';` after the last `} from '...'` line, as Nest does,
/// unless the symbol is already imported. If `path` is already imported, the symbol is
/// added to that import instead.
pub fn insert_import(content: &str, symbol: &str, path: &str) -> String {
    if has_import(content, symbol) {
        return content.to_string();
    }
    let p = Parsed::new(content);
    if let Some(imp) = p.imports.iter().find(|i| i.source == path && !i.type_only && i.namespace.is_none() && !i.names.is_empty()) {
        // Add to the existing brace list: find the closing `}` of this import.
        let close = p.toks.iter().position(|t| t.start >= imp.start && t.end <= imp.end && &p.src[t.start..t.end] == "}");
        if let Some(ci) = close {
            let prev = p.toks[ci - 1];
            let out = insert_at(content, prev.end, &format!(", {symbol}"));
            return match_eol(content, out);
        }
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"\} from ('|")"#).unwrap());
    let mut lines: Vec<&str> = content.split('\n').collect();
    let last = lines.iter().rev().find(|l| re.is_match(l)).copied();
    let idx = match last {
        Some(l) => lines.iter().position(|x| *x == l).unwrap_or(0),
        None => 0,
    };
    let line = format!("import {{ {symbol} }} from '{path}';");
    let insert_pos = (idx + 1).min(lines.len());
    lines.insert(insert_pos, &line);
    match_eol(content, lines.join("\n"))
}

/// `ModuleDeclarator.declare`: import the symbol and add it to the module metadata.
pub fn declare(content: &str, metadata: &str, symbol: &str, import_path: &str) -> String {
    let with_import = insert_import(content, symbol, import_path);
    insert_metadata(&with_import, metadata, symbol).unwrap_or(with_import)
}

/// Wraps the given identifiers in the module's `imports` array with `forwardRef(() => X)`.
pub fn wrap_forward_refs(content: &str, names: &[String]) -> String {
    let p = Parsed::new(content);
    let Some(open) = module_object(&p, "Module") else { return content.to_string() };
    let mut edits: Vec<(usize, usize, String)> = vec![];
    for prop in p.object_props(open) {
        if prop.key != "imports" || !p.is(prop.value.0, "[") {
            continue;
        }
        let close = prop.value.1 - 1;
        for (s, e) in p.split_items(prop.value.0, close) {
            if e == s + 1 && p.kind(s) == Some(Kind::Ident) && names.iter().any(|n| n == p.text(s)) {
                edits.push((p.toks[s].start, p.toks[s].end, format!("forwardRef(() => {})", p.text(s))));
            }
        }
    }
    if edits.is_empty() {
        return content.to_string();
    }
    let mut out = content.to_string();
    for (s, e, text) in edits.into_iter().rev() {
        out.replace_range(s..e, &text);
    }
    insert_import(&out, "forwardRef", "@nestjs/common")
}

/// Adds `entity` to an existing `<orm>.forFeature([...])` call in the module's imports.
/// Returns `None` when there is no such call.
pub fn add_to_for_feature(content: &str, orm_module: &str, entry: &str) -> Option<String> {
    let p = Parsed::new(content);
    let open = module_object(&p, "Module")?;
    for prop in p.object_props(open) {
        if prop.key != "imports" || !p.is(prop.value.0, "[") {
            continue;
        }
        for (s, _e) in p.split_items(prop.value.0, prop.value.1 - 1) {
            if p.is(s, orm_module) && p.is(s + 1, ".") && p.is(s + 2, "forFeature") && p.is(s + 3, "(") {
                let args_close = p.matching(s + 3)?;
                let first = p.split_items(s + 3, args_close).first().copied()?;
                if !p.is(first.0, "[") {
                    return None;
                }
                let arr_close = p.matching(first.0)?;
                let elements = p.split_items(first.0, arr_close);
                if elements.iter().any(|&(a, b)| p.slice(a, b) == entry) {
                    return Some(content.to_string());
                }
                let out = match elements.last() {
                    Some(&(_, b)) => insert_at(content, p.toks[b - 1].end, &format!(", {entry}")),
                    None => insert_at(content, p.toks[arr_close].start, entry),
                };
                return Some(match_eol(content, out));
            }
        }
    }
    None
}

/// Adds a JSON dependency to package.json content, keeping key order and sorting the
/// dependency block the way `addPackageJsonDependency` does.
pub fn add_package_dependency(content: &str, name: &str, version: &str, dev: bool) -> Option<String> {
    let mut v: serde_json::Value = serde_json::from_str(content).ok()?;
    let key = if dev { "devDependencies" } else { "dependencies" };
    let obj = v.as_object_mut()?;
    let deps = obj.entry(key).or_insert_with(|| serde_json::json!({}));
    let deps = deps.as_object_mut()?;
    if deps.contains_key(name) {
        return Some(content.to_string());
    }
    deps.insert(name.to_string(), serde_json::Value::String(version.to_string()));
    let mut entries: Vec<(String, serde_json::Value)> = deps.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    deps.clear();
    for (k, val) in entries {
        deps.insert(k, val);
    }
    let mut out = serde_json::to_string_pretty(&v).ok()?;
    out.push('\n');
    Some(match_eol(content, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "import { Module } from '@nestjs/common';\nimport { AppController } from './app.controller';\nimport { AppService } from './app.service';\n\n@Module({\n  imports: [],\n  controllers: [AppController],\n  providers: [AppService],\n})\nexport class AppModule {}\n";

    #[test]
    fn matches_nest_cli_output_for_first_resource() {
        let out = declare(APP, "imports", "UsersModule", "./users/users.module");
        assert_eq!(
            out,
            "import { Module } from '@nestjs/common';\nimport { AppController } from './app.controller';\nimport { AppService } from './app.service';\nimport { UsersModule } from './users/users.module';\n\n@Module({\n  imports: [UsersModule],\n  controllers: [AppController],\n  providers: [AppService],\n})\nexport class AppModule {}\n"
        );
        let second = declare(&out, "imports", "OrdersModule", "./orders/orders.module");
        assert!(second.contains("imports: [UsersModule, OrdersModule],"));
        // No duplicates on a repeat.
        assert_eq!(declare(&second, "imports", "OrdersModule", "./orders/orders.module"), second);
    }

    #[test]
    fn multiline_arrays_keep_their_layout() {
        let src = "@Module({\n  imports: [\n    ConfigModule.forRoot({ isGlobal: true }),\n    UsersModule,\n  ],\n})\nexport class AppModule {}\n";
        let out = insert_metadata(src, "imports", "InvoicesModule").unwrap();
        assert!(out.contains("    UsersModule,\n    InvoicesModule,\n  ],"), "{out}");
    }

    #[test]
    fn adds_missing_property_and_empty_module() {
        let out = insert_metadata("@Module({\n  imports: [A],\n})\nexport class M {}\n", "providers", "S").unwrap();
        assert_eq!(out, "@Module({\n  imports: [A],\n  providers: [S],\n})\nexport class M {}\n");
        let out = insert_metadata("@Module({})\nexport class M {}\n", "controllers", "C").unwrap();
        assert_eq!(out, "@Module({\n  controllers: [C]\n})\nexport class M {}\n");
    }

    #[test]
    fn forward_refs() {
        let src = "import { Module } from '@nestjs/common';\nimport { B } from './b';\n\n@Module({ imports: [B, C] })\nexport class A {}\n";
        let out = wrap_forward_refs(src, &["B".to_string()]);
        assert!(out.contains("import { Module, forwardRef } from '@nestjs/common';"), "{out}");
        assert!(out.contains("imports: [forwardRef(() => B), C]"));
    }

    #[test]
    fn for_feature() {
        let src = "@Module({ imports: [TypeOrmModule.forFeature([Order])] })\nexport class A {}\n";
        let out = add_to_for_feature(src, "TypeOrmModule", "Item").unwrap();
        assert!(out.contains("forFeature([Order, Item])"));
    }

    #[test]
    fn crlf_is_preserved() {
        let src = APP.replace('\n', "\r\n");
        let out = declare(&src, "imports", "UsersModule", "./users/users.module");
        assert!(!out.replace("\r\n", "").contains('\n'));
    }
}
