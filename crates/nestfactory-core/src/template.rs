//! A small EJS-subset template engine for Nest schematic templates (and team templates in
//! `.nestfactory/templates`): `<%= expr %>`, `<% if (cond) { %>`, `<% } else if (cond) { %>`,
//! `<% } else { %>`, `<% } %>`, plus `__name@fn__` path placeholders.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use regex::Regex;

use crate::naming;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Bool(bool),
    Undefined,
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Str(s) => !s.is_empty(),
            Value::Bool(b) => *b,
            Value::Undefined => false,
        }
    }

    fn as_string(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Undefined => String::new(),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

pub type Vars = HashMap<String, Value>;

#[derive(Debug, Clone)]
enum Expr {
    Lit(Value),
    Var(String),
    Call(String, Box<Expr>),
    Not(Box<Expr>),
    Eq(Box<Expr>, Box<Expr>, bool),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Debug)]
enum Node {
    Text(String),
    Out(Expr),
    If(Vec<(Option<Expr>, Vec<Node>)>),
}

pub fn render(template: &str, vars: &Vars) -> Result<String> {
    let nodes = parse(template)?;
    let mut out = String::with_capacity(template.len());
    emit(&nodes, vars, &mut out)?;
    Ok(out)
}

fn emit(nodes: &[Node], vars: &Vars, out: &mut String) -> Result<()> {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Out(e) => out.push_str(&eval(e, vars)?.as_string()),
            Node::If(branches) => {
                for (cond, body) in branches {
                    let take = match cond {
                        Some(c) => eval(c, vars)?.truthy(),
                        None => true,
                    };
                    if take {
                        emit(body, vars, out)?;
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

fn parse(template: &str) -> Result<Vec<Node>> {
    // Stack of open `if` blocks: (branches so far, current branch condition, current body).
    struct Open {
        branches: Vec<(Option<Expr>, Vec<Node>)>,
        cond: Option<Expr>,
        body: Vec<Node>,
    }
    let mut root: Vec<Node> = vec![];
    let mut stack: Vec<Open> = vec![];
    let push = |stack: &mut Vec<Open>, root: &mut Vec<Node>, n: Node| match stack.last_mut() {
        Some(o) => o.body.push(n),
        None => root.push(n),
    };
    let mut rest = template;
    while let Some(start) = rest.find("<%") {
        if start > 0 {
            push(&mut stack, &mut root, Node::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        let end = after.find("%>").ok_or_else(|| anyhow!("unclosed <% tag"))?;
        let tag = &after[..end];
        rest = &after[end + 2..];
        if let Some(expr) = tag.strip_prefix('=').or_else(|| tag.strip_prefix('-')) {
            push(&mut stack, &mut root, Node::Out(parse_expr(expr)?));
            continue;
        }
        let mut code = tag.trim();
        let closing = code.starts_with('}');
        if closing {
            code = code[1..].trim();
        }
        if let Some(e) = code.strip_prefix("else if") {
            let open = stack.last_mut().ok_or_else(|| anyhow!("else without if"))?;
            let body = std::mem::take(&mut open.body);
            let cond = open.cond.take();
            open.branches.push((cond, body));
            open.cond = Some(parse_condition(e)?);
        } else if code.starts_with("else") {
            let open = stack.last_mut().ok_or_else(|| anyhow!("else without if"))?;
            let body = std::mem::take(&mut open.body);
            let cond = open.cond.take();
            open.branches.push((cond, body));
            open.cond = None;
        } else {
            if closing {
                let mut open = stack.pop().ok_or_else(|| anyhow!("unbalanced }}"))?;
                open.branches.push((open.cond.take(), std::mem::take(&mut open.body)));
                push(&mut stack, &mut root, Node::If(open.branches));
            }
            if let Some(e) = code.strip_prefix("if") {
                stack.push(Open { branches: vec![], cond: Some(parse_condition(e)?), body: vec![] });
            } else if !code.is_empty() && code != "{" {
                bail!("unsupported template code: {code}");
            }
        }
    }
    if !rest.is_empty() {
        push(&mut stack, &mut root, Node::Text(rest.to_string()));
    }
    if !stack.is_empty() {
        bail!("unclosed if block");
    }
    Ok(root)
}

/// `(cond) {` → cond
fn parse_condition(s: &str) -> Result<Expr> {
    let s = s.trim().trim_end_matches('{').trim();
    parse_expr(s)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Op(&'static str),
}

fn lex(s: &str) -> Result<Vec<Tok>> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = vec![];
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] == b'.') {
                i += 1;
            }
            out.push(Tok::Ident(s[st..i].to_string()));
            continue;
        }
        if c == b'\'' || c == b'"' {
            let q = c;
            i += 1;
            let st = i;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            out.push(Tok::Str(s[st..i].to_string()));
            i += 1;
            continue;
        }
        let ops = ["===", "!==", "==", "!=", "&&", "||", "!", "(", ")", "?", ":"];
        let op = ops.iter().find(|o| s[i..].starts_with(**o)).ok_or_else(|| anyhow!("unexpected '{}' in template expression", c as char))?;
        out.push(Tok::Op(op));
        i += op.len();
    }
    Ok(out)
}

fn parse_expr(s: &str) -> Result<Expr> {
    let toks = lex(s)?;
    let mut p = 0;
    let e = ternary(&toks, &mut p)?;
    if p != toks.len() {
        bail!("trailing tokens in template expression: {s}");
    }
    Ok(e)
}

fn ternary(t: &[Tok], p: &mut usize) -> Result<Expr> {
    let c = or(t, p)?;
    if t.get(*p) == Some(&Tok::Op("?")) {
        *p += 1;
        let a = ternary(t, p)?;
        if t.get(*p) != Some(&Tok::Op(":")) {
            bail!("expected ':'");
        }
        *p += 1;
        let b = ternary(t, p)?;
        return Ok(Expr::Ternary(Box::new(c), Box::new(a), Box::new(b)));
    }
    Ok(c)
}

fn or(t: &[Tok], p: &mut usize) -> Result<Expr> {
    let mut l = and(t, p)?;
    while t.get(*p) == Some(&Tok::Op("||")) {
        *p += 1;
        l = Expr::Or(Box::new(l), Box::new(and(t, p)?));
    }
    Ok(l)
}

fn and(t: &[Tok], p: &mut usize) -> Result<Expr> {
    let mut l = eq(t, p)?;
    while t.get(*p) == Some(&Tok::Op("&&")) {
        *p += 1;
        l = Expr::And(Box::new(l), Box::new(eq(t, p)?));
    }
    Ok(l)
}

fn eq(t: &[Tok], p: &mut usize) -> Result<Expr> {
    let l = unary(t, p)?;
    match t.get(*p) {
        Some(Tok::Op(op @ ("===" | "==" | "!==" | "!="))) => {
            *p += 1;
            let r = unary(t, p)?;
            Ok(Expr::Eq(Box::new(l), Box::new(r), matches!(*op, "===" | "==")))
        }
        _ => Ok(l),
    }
}

fn unary(t: &[Tok], p: &mut usize) -> Result<Expr> {
    if t.get(*p) == Some(&Tok::Op("!")) {
        *p += 1;
        return Ok(Expr::Not(Box::new(unary(t, p)?)));
    }
    primary(t, p)
}

fn primary(t: &[Tok], p: &mut usize) -> Result<Expr> {
    match t.get(*p).cloned() {
        Some(Tok::Str(s)) => {
            *p += 1;
            Ok(Expr::Lit(Value::Str(s)))
        }
        Some(Tok::Op("(")) => {
            *p += 1;
            let e = ternary(t, p)?;
            if t.get(*p) != Some(&Tok::Op(")")) {
                bail!("expected ')'");
            }
            *p += 1;
            Ok(e)
        }
        Some(Tok::Ident(name)) => {
            *p += 1;
            if t.get(*p) == Some(&Tok::Op("(")) {
                *p += 1;
                let arg = ternary(t, p)?;
                if t.get(*p) != Some(&Tok::Op(")")) {
                    bail!("expected ')' after argument");
                }
                *p += 1;
                return Ok(Expr::Call(name, Box::new(arg)));
            }
            Ok(match name.as_str() {
                "true" => Expr::Lit(Value::Bool(true)),
                "false" => Expr::Lit(Value::Bool(false)),
                _ => Expr::Var(name),
            })
        }
        other => bail!("unexpected token in template expression: {other:?}"),
    }
}

fn eval(e: &Expr, vars: &Vars) -> Result<Value> {
    Ok(match e {
        Expr::Lit(v) => v.clone(),
        Expr::Var(n) => vars.get(n).cloned().unwrap_or(Value::Undefined),
        Expr::Not(x) => Value::Bool(!eval(x, vars)?.truthy()),
        Expr::Eq(a, b, want) => Value::Bool((eval(a, vars)? == eval(b, vars)?) == *want),
        Expr::And(a, b) => {
            let l = eval(a, vars)?;
            if l.truthy() { eval(b, vars)? } else { l }
        }
        Expr::Or(a, b) => {
            let l = eval(a, vars)?;
            if l.truthy() { l } else { eval(b, vars)? }
        }
        Expr::Ternary(c, a, b) => {
            if eval(c, vars)?.truthy() { eval(a, vars)? } else { eval(b, vars)? }
        }
        Expr::Call(f, arg) => Value::Str(apply_fn(f, &eval(arg, vars)?.as_string())?),
    })
}

pub fn apply_fn(f: &str, s: &str) -> Result<String> {
    Ok(match f {
        "classify" => naming::classify(s),
        "dasherize" => naming::dasherize(s),
        "camelize" => naming::camelize(s),
        "capitalize" => naming::capitalize(s),
        "decamelize" => naming::decamelize(s),
        "underscore" => naming::underscore(s),
        "singular" => naming::singular(s),
        "lowercased" => naming::lowercased(s),
        "ent" => format!("{s}.entity"),
        _ => bail!("unknown template function {f}"),
    })
}

/// Expands `__name__`, `__name@singular@ent__` style placeholders in a template path.
pub fn render_path(path: &str, vars: &Vars) -> Result<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"__([A-Za-z]+)((?:@[A-Za-z]+)*)__").unwrap());
    let mut err = None;
    let out = re.replace_all(path, |c: &regex::Captures| {
        let mut v = vars.get(&c[1]).map(|v| v.as_string()).unwrap_or_default();
        for f in c[2].split('@').filter(|f| !f.is_empty()) {
            match apply_fn(f, &v) {
                Ok(x) => v = x,
                Err(e) => err = Some(e),
            }
        }
        v
    });
    match err {
        Some(e) => Err(e),
        None => Ok(out.into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.insert("name".into(), "invoices".into());
        v.insert("type".into(), "rest".into());
        v.insert("crud".into(), true.into());
        v.insert("isEsm".into(), false.into());
        v
    }

    #[test]
    fn renders_outputs_and_branches() {
        let t = "<% if (crud && type === 'rest') { %>A<%= classify(name) %><%\n} else if (type === 'ws') { %>B<% } else { %>C<% } %>x<%= isEsm ? '.js' : '' %>";
        assert_eq!(render(t, &vars()).unwrap(), "AInvoicesx");
    }

    #[test]
    fn nested_calls() {
        let t = "<%= lowercased(singular(classify(name))) %>";
        assert_eq!(render(t, &vars()).unwrap(), "invoice");
    }

    #[test]
    fn paths() {
        assert_eq!(render_path("entities/__name@singular@ent__.ts", &vars()).unwrap(), "entities/invoice.entity.ts");
        assert_eq!(render_path("__name__.controller.ts", &vars()).unwrap(), "invoices.controller.ts");
    }
}
