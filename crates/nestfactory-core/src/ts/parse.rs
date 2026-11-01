//! Structural parser over the token stream: imports, classes, decorators, constructor
//! parameters, methods and properties. It understands just enough TypeScript to read Nest
//! metadata and to locate exact byte positions for edits.

use super::lexer::{tokenize, Kind, LineIndex, Token};

const NONE: u32 = u32::MAX;

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub source: String,
    /// `(imported, local)` pairs from `import { a as b }`.
    pub names: Vec<(String, String)>,
    pub default: Option<String>,
    pub namespace: Option<String>,
    pub type_only: bool,
    pub start: usize,
    pub end: usize,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub line: u32,
    /// Token indices of the `(` and `)` around the arguments.
    pub args: Option<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub decorators: Vec<Decorator>,
    pub type_text: String,
    pub type_name: Option<String>,
    pub type_args: Vec<String>,
    pub optional: bool,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Method {
    pub name: String,
    pub decorators: Vec<Decorator>,
    pub params: Vec<Param>,
    pub is_static: bool,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub decorators: Vec<Decorator>,
    pub type_text: String,
    pub optional: bool,
    pub has_initializer: bool,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    pub exported: bool,
    pub decorators: Vec<Decorator>,
    pub extends: Option<String>,
    pub implements: Vec<String>,
    pub ctor_params: Vec<Param>,
    /// Token indices of the constructor's `(` and `)`, if it has one.
    pub ctor_group: Option<(usize, usize)>,
    pub methods: Vec<Method>,
    pub properties: Vec<Property>,
    pub line: u32,
    pub start: usize,
    /// Token indices of the class body braces.
    pub body: (usize, usize),
}

/// A property of an object literal: `key: value`, shorthand `key`, or a spread.
#[derive(Debug, Clone)]
pub struct Prop {
    pub key: String,
    pub key_tok: usize,
    /// Token range `[start, end)` of the value.
    pub value: (usize, usize),
    pub spread: bool,
}

#[derive(Debug, Clone)]
pub struct Parsed {
    pub src: String,
    pub toks: Vec<Token>,
    matching: Vec<u32>,
    pub lines: LineIndex,
    pub imports: Vec<ImportDecl>,
    pub classes: Vec<ClassDecl>,
}

impl Parsed {
    pub fn new(src: impl Into<String>) -> Self {
        let src = src.into();
        let toks = tokenize(&src);
        let matching = match_brackets(&src, &toks);
        let lines = LineIndex::new(&src);
        let mut p = Parsed { src, toks, matching, lines, imports: vec![], classes: vec![] };
        p.parse_top_level();
        p
    }

    // ---- token helpers -------------------------------------------------------------------

    pub fn text(&self, i: usize) -> &str {
        self.toks.get(i).map(|t| &self.src[t.start..t.end]).unwrap_or("")
    }

    pub fn is(&self, i: usize, s: &str) -> bool {
        self.toks.get(i).is_some_and(|t| &self.src[t.start..t.end] == s)
    }

    pub fn kind(&self, i: usize) -> Option<Kind> {
        self.toks.get(i).map(|t| t.kind)
    }

    pub fn line_of_tok(&self, i: usize) -> u32 {
        self.toks.get(i).map(|t| self.lines.line(t.start)).unwrap_or(0)
    }

    /// The matching bracket for `(`, `[` or `{` at `i`.
    pub fn matching(&self, i: usize) -> Option<usize> {
        self.matching.get(i).copied().filter(|&m| m != NONE).map(|m| m as usize)
    }

    /// Source text covered by the token range `[s, e)`.
    pub fn slice(&self, s: usize, e: usize) -> &str {
        if s >= e || e > self.toks.len() {
            return "";
        }
        &self.src[self.toks[s].start..self.toks[e - 1].end]
    }

    pub fn string_value(&self, i: usize) -> Option<String> {
        let t = self.toks.get(i)?;
        if t.kind != Kind::Str && t.kind != Kind::Template {
            return None;
        }
        let raw = &self.src[t.start..t.end];
        if raw.len() < 2 {
            return None;
        }
        Some(raw[1..raw.len() - 1].to_string())
    }

    /// Skips a balanced group starting at `i` if it is an opening bracket.
    fn skip(&self, i: usize) -> usize {
        match self.matching(i) {
            Some(m) if m > i => m + 1,
            _ => i + 1,
        }
    }

    /// Splits the contents of a bracket group into comma separated token ranges.
    pub fn split_items(&self, open: usize, close: usize) -> Vec<(usize, usize)> {
        let mut items = vec![];
        let mut s = open + 1;
        let mut i = open + 1;
        while i < close {
            if self.is(i, ",") && self.kind(i) == Some(Kind::Punct) {
                if i > s {
                    items.push((s, i));
                }
                i += 1;
                s = i;
                continue;
            }
            i = self.skip(i);
        }
        if close > s {
            items.push((s, close));
        }
        items
    }

    /// Reads an object literal whose `{` is at `open`.
    pub fn object_props(&self, open: usize) -> Vec<Prop> {
        let Some(close) = self.matching(open) else { return vec![] };
        if !self.is(open, "{") {
            return vec![];
        }
        let mut out = vec![];
        for (s, e) in self.split_items(open, close) {
            if self.is(s, "...") {
                out.push(Prop { key: "...".into(), key_tok: s, value: (s + 1, e), spread: true });
                continue;
            }
            let key = match self.kind(s) {
                Some(Kind::Ident) | Some(Kind::Num) => self.text(s).to_string(),
                Some(Kind::Str) => self.string_value(s).unwrap_or_default(),
                _ => continue,
            };
            if s + 1 < e && self.is(s + 1, ":") {
                out.push(Prop { key, key_tok: s, value: (s + 2, e), spread: false });
            } else {
                out.push(Prop { key, key_tok: s, value: (s, e), spread: false });
            }
        }
        out
    }

    /// The first argument token range of a decorator call.
    pub fn decorator_arg(&self, d: &Decorator, index: usize) -> Option<(usize, usize)> {
        let (open, close) = d.args?;
        self.split_items(open, close).get(index).copied()
    }

    pub fn decorator_string_arg(&self, d: &Decorator) -> Option<String> {
        let (s, e) = self.decorator_arg(d, 0)?;
        if e == s + 1 {
            return self.string_value(s);
        }
        None
    }

    // ---- top level -----------------------------------------------------------------------

    fn parse_top_level(&mut self) {
        let n = self.toks.len();
        let mut i = 0;
        let mut pending: Vec<Decorator> = vec![];
        let mut imports = vec![];
        let mut classes = vec![];
        while i < n {
            let t = self.toks[i];
            match (t.kind, self.text(i)) {
                (Kind::Punct, "@") if self.kind(i + 1) == Some(Kind::Ident) => {
                    let (d, next) = self.parse_decorator(i);
                    pending.push(d);
                    i = next;
                }
                (Kind::Ident, "import")
                    if !self.is(i + 1, "(") && !self.is(i + 1, ".") && !self.is_member_access(i) =>
                {
                    let (decl, next) = self.parse_import(i);
                    if let Some(d) = decl {
                        imports.push(d);
                    }
                    i = next;
                }
                (Kind::Ident, "class") if !self.is_member_access(i) => {
                    let (c, next) = self.parse_class(i, std::mem::take(&mut pending));
                    if let Some(c) = c {
                        classes.push(c);
                    }
                    i = next;
                }
                (Kind::Ident, "export" | "default" | "abstract" | "declare") => i += 1,
                (Kind::Punct, "(" | "[" | "{") => {
                    pending.clear();
                    i = self.skip(i);
                }
                _ => {
                    pending.clear();
                    i += 1;
                }
            }
        }
        self.imports = imports;
        self.classes = classes;
    }

    fn is_member_access(&self, i: usize) -> bool {
        i > 0 && (self.is(i - 1, ".") || self.is(i - 1, "?."))
    }

    fn parse_decorator(&self, at: usize) -> (Decorator, usize) {
        let mut i = at + 1;
        let mut name = self.text(i).to_string();
        i += 1;
        while self.is(i, ".") && self.kind(i + 1) == Some(Kind::Ident) {
            name.push('.');
            name.push_str(self.text(i + 1));
            i += 2;
        }
        let mut args = None;
        let mut end = self.toks[i - 1].end;
        if self.is(i, "(") {
            if let Some(m) = self.matching(i) {
                args = Some((i, m));
                end = self.toks[m].end;
                i = m + 1;
            }
        }
        let start = self.toks[at].start;
        (Decorator { name, start, end, line: self.lines.line(start), args }, i)
    }

    fn parse_import(&self, at: usize) -> (Option<ImportDecl>, usize) {
        let n = self.toks.len();
        let mut i = at + 1;
        let mut decl = ImportDecl {
            source: String::new(),
            names: vec![],
            default: None,
            namespace: None,
            type_only: false,
            start: self.toks[at].start,
            end: self.toks[at].end,
            line: self.line_of_tok(at),
        };
        if self.is(i, "type") && !self.is(i + 1, "from") && !self.is(i + 1, ",") {
            decl.type_only = true;
            i += 1;
        }
        loop {
            if i >= n {
                return (None, i);
            }
            match (self.kind(i), self.text(i)) {
                (Some(Kind::Str), _) => {
                    decl.source = self.string_value(i).unwrap_or_default();
                    i += 1;
                    break;
                }
                (Some(Kind::Punct), "{") => {
                    let Some(close) = self.matching(i) else { return (None, i + 1) };
                    for (s, e) in self.split_items(i, close) {
                        let mut parts: Vec<&str> = (s..e).map(|k| self.text(k)).collect();
                        if parts.first() == Some(&"type") && parts.len() > 1 {
                            parts.remove(0);
                        }
                        match parts.as_slice() {
                            [a] => decl.names.push((a.to_string(), a.to_string())),
                            [a, "as", b] => decl.names.push((a.to_string(), b.to_string())),
                            _ => {}
                        }
                    }
                    i = close + 1;
                }
                (Some(Kind::Punct), "*") => {
                    if self.is(i + 1, "as") {
                        decl.namespace = Some(self.text(i + 2).to_string());
                    }
                    i += 3;
                }
                (Some(Kind::Punct), ",") => i += 1,
                (Some(Kind::Ident), "from") => i += 1,
                (Some(Kind::Ident), _) if self.is(i + 1, "=") => {
                    // `import x = require('y')`: not an ES import.
                    return (None, i + 1);
                }
                (Some(Kind::Ident), word) => {
                    decl.default = Some(word.to_string());
                    i += 1;
                }
                _ => return (None, i + 1),
            }
        }
        if self.is(i, ";") {
            i += 1;
        }
        decl.end = self.toks[i - 1].end;
        (Some(decl), i)
    }

    fn parse_class(&self, at: usize, decorators: Vec<Decorator>) -> (Option<ClassDecl>, usize) {
        let mut exported = false;
        let mut k = at;
        while k > 0 {
            k -= 1;
            match self.text(k) {
                "export" => {
                    exported = true;
                    break;
                }
                "default" | "abstract" | "declare" => continue,
                _ => break,
            }
        }
        let mut i = at + 1;
        let name = if self.kind(i) == Some(Kind::Ident)
            && !matches!(self.text(i), "extends" | "implements")
        {
            i += 1;
            self.text(i - 1).to_string()
        } else {
            "default".to_string()
        };
        if self.is(i, "<") {
            i = self.skip_angles(i);
        }
        let mut extends = None;
        let mut implements = vec![];
        if self.is(i, "extends") {
            let s = i + 1;
            i = s;
            while i < self.toks.len() && !self.is(i, "implements") && !self.is(i, "{") {
                i = if self.is(i, "<") { self.skip_angles(i) } else { self.skip(i) };
            }
            extends = Some(self.slice(s, i).to_string());
        }
        if self.is(i, "implements") {
            i += 1;
            let mut cur = String::new();
            while i < self.toks.len() && !self.is(i, "{") {
                if self.is(i, ",") {
                    implements.push(std::mem::take(&mut cur));
                    i += 1;
                } else if self.is(i, "<") {
                    i = self.skip_angles(i);
                } else {
                    cur.push_str(self.text(i));
                    i += 1;
                }
            }
            if !cur.is_empty() {
                implements.push(cur);
            }
        }
        if !self.is(i, "{") {
            return (None, i);
        }
        let Some(close) = self.matching(i) else { return (None, i + 1) };
        let start = decorators.first().map(|d| d.start).unwrap_or(self.toks[at].start);
        let mut class = ClassDecl {
            name,
            exported,
            decorators,
            extends,
            implements,
            ctor_params: vec![],
            ctor_group: None,
            methods: vec![],
            properties: vec![],
            line: self.line_of_tok(at),
            start,
            body: (i, close),
        };
        self.parse_members(&mut class, i, close);
        (Some(class), close + 1)
    }

    fn skip_angles(&self, at: usize) -> usize {
        let mut depth = 0i32;
        let mut i = at;
        while i < self.toks.len() {
            match self.text(i) {
                "<" => depth += 1,
                ">" => {
                    depth -= 1;
                    if depth <= 0 {
                        return i + 1;
                    }
                }
                "(" | "[" | "{" => {
                    i = self.skip(i);
                    continue;
                }
                ";" => return i,
                _ => {}
            }
            i += 1;
        }
        i
    }

    fn parse_members(&self, class: &mut ClassDecl, open: usize, close: usize) {
        const MODIFIERS: &[&str] = &[
            "public", "private", "protected", "readonly", "static", "async", "abstract",
            "override", "declare", "accessor",
        ];
        let mut i = open + 1;
        let mut decorators: Vec<Decorator> = vec![];
        while i < close {
            if self.is(i, ";") || self.is(i, ",") {
                i += 1;
                continue;
            }
            if self.is(i, "@") && self.kind(i + 1) == Some(Kind::Ident) {
                let (d, next) = self.parse_decorator(i);
                decorators.push(d);
                i = next;
                continue;
            }
            let member_start = i;
            let mut is_static = false;
            while self.kind(i) == Some(Kind::Ident)
                && (MODIFIERS.contains(&self.text(i)) || matches!(self.text(i), "get" | "set"))
                && !matches!(self.text(i + 1), "(" | ":" | "=" | ";" | "?" | "!" | "<")
                && i + 1 < close
            {
                if self.is(i, "static") {
                    is_static = true;
                }
                i += 1;
            }
            if self.is(i, "*") {
                i += 1;
            }
            let name = match self.kind(i) {
                Some(Kind::Ident) | Some(Kind::Num) => self.text(i).to_string(),
                Some(Kind::Str) => self.string_value(i).unwrap_or_default(),
                Some(Kind::Punct) if self.is(i, "[") => {
                    let s = i;
                    i = self.skip(i) - 1;
                    self.slice(s, i + 1).to_string()
                }
                _ => {
                    i = self.skip(i);
                    decorators.clear();
                    continue;
                }
            };
            let line = self.line_of_tok(member_start.max(i));
            i += 1;
            let mut optional = false;
            if self.is(i, "?") || self.is(i, "!") {
                optional = self.is(i, "?");
                i += 1;
            }
            if self.is(i, "<") {
                i = self.skip_angles(i);
            }
            if self.is(i, "(") {
                let Some(pclose) = self.matching(i) else { break };
                let params = self.parse_params(i, pclose);
                let group = (i, pclose);
                i = pclose + 1;
                if self.is(i, ":") {
                    i = self.skip_type(i + 1, close, true);
                }
                if self.is(i, "{") {
                    i = self.skip(i);
                } else if self.is(i, ";") {
                    i += 1;
                }
                let decs = std::mem::take(&mut decorators);
                if name == "constructor" {
                    class.ctor_params = params;
                    class.ctor_group = Some(group);
                } else {
                    class.methods.push(Method { name, decorators: decs, params, is_static, line });
                }
            } else {
                let mut type_text = String::new();
                if self.is(i, ":") {
                    let s = i + 1;
                    i = self.skip_type(s, close, false);
                    type_text = self.slice(s, i).to_string();
                }
                let mut has_initializer = false;
                if self.is(i, "=") {
                    has_initializer = true;
                    i = self.skip_expression(i + 1, close);
                }
                class.properties.push(Property {
                    name,
                    decorators: std::mem::take(&mut decorators),
                    type_text,
                    optional,
                    has_initializer,
                    line,
                });
            }
        }
    }

    /// Skips a type annotation. For return types, stops at the body `{`.
    fn skip_type(&self, mut i: usize, limit: usize, return_type: bool) -> usize {
        let mut angle = 0i32;
        let start = i;
        while i < limit {
            let t = self.text(i);
            match t {
                "<" => angle += 1,
                ">" => angle -= 1,
                "=>" => {}
                "{" if angle <= 0 => {
                    let prev = if i > start { self.text(i - 1) } else { ":" };
                    let starts_type = i == start || matches!(prev, "|" | "&" | "=>" | "," | "<" | ":" | "(");
                    if return_type && !starts_type {
                        return i;
                    }
                    i = self.skip(i);
                    continue;
                }
                "(" | "[" | "{" => {
                    i = self.skip(i);
                    continue;
                }
                "=" | ";" | "," | ")" if angle <= 0 => return i,
                _ => {}
            }
            if !return_type && angle <= 0 && self.ends_statement(i) {
                return i + 1;
            }
            i += 1;
        }
        i
    }

    fn skip_expression(&self, mut i: usize, limit: usize) -> usize {
        while i < limit {
            if self.is(i, ";") {
                return i;
            }
            let next = self.skip(i);
            if self.ends_statement(next - 1) {
                return next;
            }
            i = next;
        }
        i
    }

    /// Automatic semicolon insertion heuristic: does the statement end after token `i`?
    fn ends_statement(&self, i: usize) -> bool {
        let Some(t) = self.toks.get(i) else { return true };
        let Some(next) = self.toks.get(i + 1) else { return true };
        if self.lines.line(t.end) == self.lines.line(next.start) {
            return false;
        }
        let cur = self.text(i);
        let continues = matches!(
            cur,
            "=" | "," | "|" | "&" | "+" | "-" | "*" | "/" | "?" | ":" | "=>" | "." | "(" | "[" | "{" | "<" | "&&" | "||" | "??"
        );
        let next_text = self.text(i + 1);
        let next_continues = matches!(next_text, "." | "?." | "|" | "&" | "?" | ":" | ")" | "]" | "&&" | "||" | "??");
        !continues && !next_continues
    }

    fn parse_params(&self, open: usize, close: usize) -> Vec<Param> {
        let mut out = vec![];
        for (s, e) in self.split_items(open, close) {
            let mut i = s;
            let mut decorators = vec![];
            while i < e && self.is(i, "@") {
                let (d, next) = self.parse_decorator(i);
                decorators.push(d);
                i = next;
            }
            while i + 1 < e
                && matches!(self.text(i), "public" | "private" | "protected" | "readonly" | "override")
            {
                i += 1;
            }
            if self.is(i, "...") {
                i += 1;
            }
            let line = self.line_of_tok(i);
            let name = if self.is(i, "{") || self.is(i, "[") {
                let n = self.skip(i);
                let txt = self.slice(i, n).to_string();
                i = n;
                txt
            } else {
                i += 1;
                self.text(i - 1).to_string()
            };
            let mut optional = false;
            if self.is(i, "?") {
                optional = true;
                i += 1;
            }
            let mut type_text = String::new();
            let mut type_name = None;
            let mut type_args = vec![];
            if self.is(i, ":") {
                let ts = i + 1;
                let mut te = ts;
                let mut angle = 0;
                while te < e {
                    match self.text(te) {
                        "=" if angle == 0 => break,
                        "<" => angle += 1,
                        ">" => angle -= 1,
                        _ => {}
                    }
                    te = self.skip(te);
                }
                type_text = self.slice(ts, te).to_string();
                if self.kind(ts) == Some(Kind::Ident) && !self.is(ts, "typeof") {
                    let mut k = ts;
                    let mut head = self.text(k).to_string();
                    while self.is(k + 1, ".") && self.kind(k + 2) == Some(Kind::Ident) {
                        k += 2;
                        head = self.text(k).to_string();
                    }
                    type_name = Some(head);
                    if self.is(k + 1, "<") {
                        let mut j = k + 2;
                        while j < te && !self.is(j, ">") {
                            if self.kind(j) == Some(Kind::Ident) {
                                type_args.push(self.text(j).to_string());
                            }
                            j += 1;
                        }
                    }
                }
            }
            out.push(Param { name, decorators, type_text, type_name, type_args, optional, line });
        }
        out
    }
}

fn match_brackets(src: &str, toks: &[Token]) -> Vec<u32> {
    let mut matching = vec![NONE; toks.len()];
    let mut stack: Vec<(usize, u8)> = vec![];
    for (i, t) in toks.iter().enumerate() {
        if t.kind != Kind::Punct || t.end - t.start != 1 {
            continue;
        }
        let c = src.as_bytes()[t.start];
        match c {
            b'(' | b'[' | b'{' => stack.push((i, c)),
            b')' | b']' | b'}' => {
                let want = match c {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                // Pop until the matching opener; unmatched openers stay unmatched.
                while let Some((j, open)) = stack.pop() {
                    if open == want {
                        matching[i] = j as u32;
                        matching[j] = i as u32;
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    matching
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER: &str = r#"
import { Body, Controller, Get, Param, Post } from '@nestjs/common';
import type { Request } from 'express';
import * as path from 'path';
import Default, { other as alias } from './x';
import { OrdersService } from './orders.service';

@ApiTags('orders')
@Controller('orders')
export class OrdersController {
  private readonly log = new Logger(OrdersController.name);
  name?: string;

  constructor(
    private readonly ordersService: OrdersService,
    @InjectRepository(Order) private repo: Repository<Order>,
    @Inject(forwardRef(() => PaymentsService)) private payments: PaymentsService,
  ) {}

  @Post()
  create(@Body() dto: CreateOrderDto): Promise<{ id: number }> {
    return this.ordersService.create(dto);
  }

  @Get(':id')
  findOne(@Param('id') id: string) {
    const re = /a{2}/;
    return `${id}`;
  }
}
"#;

    #[test]
    fn parses_imports() {
        let p = Parsed::new(CONTROLLER);
        assert_eq!(p.imports.len(), 5);
        assert_eq!(p.imports[0].source, "@nestjs/common");
        assert_eq!(p.imports[0].names.len(), 5);
        assert!(p.imports[1].type_only);
        assert_eq!(p.imports[2].namespace.as_deref(), Some("path"));
        assert_eq!(p.imports[3].default.as_deref(), Some("Default"));
        assert_eq!(p.imports[3].names[0], ("other".to_string(), "alias".to_string()));
    }

    #[test]
    fn parses_class_metadata() {
        let p = Parsed::new(CONTROLLER);
        assert_eq!(p.classes.len(), 1);
        let c = &p.classes[0];
        assert_eq!(c.name, "OrdersController");
        assert!(c.exported);
        let decs: Vec<_> = c.decorators.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(decs, vec!["ApiTags", "Controller"]);
        assert_eq!(p.decorator_string_arg(&c.decorators[1]).as_deref(), Some("orders"));
        assert_eq!(c.ctor_params.len(), 3);
        assert_eq!(c.ctor_params[0].type_name.as_deref(), Some("OrdersService"));
        assert_eq!(c.ctor_params[1].type_name.as_deref(), Some("Repository"));
        assert_eq!(c.ctor_params[1].type_args, vec!["Order"]);
        assert_eq!(c.ctor_params[1].decorators[0].name, "InjectRepository");
        let methods: Vec<_> = c.methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(methods, vec!["create", "findOne"]);
        assert_eq!(c.methods[1].decorators[0].name, "Get");
        assert_eq!(c.properties.len(), 2);
        assert!(c.properties[1].optional);
    }

    #[test]
    fn reads_module_object() {
        let src = "@Module({ imports: [A, B.forRoot({ x: 1 }), forwardRef(() => C)], providers: [S, { provide: 'T', useValue: 1 }], ...rest })\nexport class M {}";
        let p = Parsed::new(src);
        let d = &p.classes[0].decorators[0];
        let (s, _) = p.decorator_arg(d, 0).unwrap();
        let props = p.object_props(s);
        assert_eq!(props.len(), 3);
        assert_eq!(props[0].key, "imports");
        assert!(props[2].spread);
        let arr = props[0].value.0;
        let items = p.split_items(arr, p.matching(arr).unwrap());
        assert_eq!(items.len(), 3);
        assert_eq!(p.slice(items[1].0, items[1].1), "B.forRoot({ x: 1 })");
    }
}
