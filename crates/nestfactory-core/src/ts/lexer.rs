//! A small TypeScript tokenizer.
//!
//! It does not build an AST. It produces tokens with byte spans, skips comments, and keeps
//! strings, template literals and regex literals as single tokens. That is all the decorator
//! parser needs, and it keeps scanning a large project well under the 1 s refresh budget.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ident,
    Str,
    Template,
    Num,
    Regex,
    Punct,
}

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

const REGEX_KEYWORDS: &[&str] = &[
    "return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do",
    "else", "yield", "await",
];

pub fn tokenize(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let len = b.len();
    let mut toks: Vec<Token> = Vec::with_capacity(len / 4);
    let mut i = 0;
    if src.starts_with("#!") {
        i = skip_line(b, 0);
    }
    while i < len {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < len {
            if b[i + 1] == b'/' {
                i = skip_line(b, i);
                continue;
            }
            if b[i + 1] == b'*' {
                i = skip_block(b, i);
                continue;
            }
            if regex_allowed(src, &toks) {
                if let Some(end) = scan_regex(b, i) {
                    toks.push(Token { kind: Kind::Regex, start: i, end });
                    i = end;
                    continue;
                }
            }
        }
        if is_ident_start(c) {
            let s = i;
            i += 1;
            while i < len && is_ident_part(b[i]) {
                i += 1;
            }
            toks.push(Token { kind: Kind::Ident, start: s, end: i });
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < len && b[i + 1].is_ascii_digit()) {
            let s = i;
            i += 1;
            while i < len && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                i += 1;
            }
            toks.push(Token { kind: Kind::Num, start: s, end: i });
            continue;
        }
        if c == b'\'' || c == b'"' {
            let end = scan_string(b, i, c);
            toks.push(Token { kind: Kind::Str, start: i, end });
            i = end;
            continue;
        }
        if c == b'`' {
            let end = scan_template(b, i);
            toks.push(Token { kind: Kind::Template, start: i, end });
            i = end;
            continue;
        }
        let n = punct_len(b, i);
        toks.push(Token { kind: Kind::Punct, start: i, end: i + n });
        i += n;
    }
    toks
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c == b'#' || c >= 0x80
}

fn is_ident_part(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

fn punct_len(b: &[u8], i: usize) -> usize {
    let rest = &b[i..];
    for p in ["...", "===", "!=="] {
        if rest.starts_with(p.as_bytes()) {
            return 3;
        }
    }
    for p in ["=>", "==", "!=", "&&", "||", "??"] {
        if rest.starts_with(p.as_bytes()) {
            return 2;
        }
    }
    if rest.starts_with(b"?.") && !rest.get(2).is_some_and(|c| c.is_ascii_digit()) {
        return 2;
    }
    1
}

fn skip_line(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_block(b: &[u8], mut i: usize) -> usize {
    i += 2;
    while i + 1 < b.len() {
        if b[i] == b'*' && b[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    b.len()
}

fn scan_string(b: &[u8], mut i: usize, quote: u8) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

fn scan_template(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'`' => return i + 1,
            b'$' if b.get(i + 1) == Some(&b'{') => i = skip_braced(b, i + 2),
            _ => i += 1,
        }
    }
    b.len()
}

/// Skips a `${ ... }` expression inside a template literal; returns the index after `}`.
fn skip_braced(b: &[u8], mut i: usize) -> usize {
    let mut depth = 1;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'\'' | b'"' => {
                i = scan_string(b, i, b[i]);
                continue;
            }
            b'`' => {
                i = scan_template(b, i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = skip_line(b, i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = skip_block(b, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn regex_allowed(src: &str, toks: &[Token]) -> bool {
    let Some(last) = toks.last() else { return true };
    let text = &src[last.start..last.end];
    match last.kind {
        Kind::Punct => !matches!(text, ")" | "]"),
        Kind::Ident => REGEX_KEYWORDS.contains(&text),
        _ => false,
    }
}

fn scan_regex(b: &[u8], mut i: usize) -> Option<usize> {
    i += 1;
    let mut in_class = false;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return None,
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return Some(i);
            }
            _ => i += 1,
        }
    }
    None
}

/// Maps byte offsets to 1-based line numbers.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0];
        for (i, c) in src.bytes().enumerate() {
            if c == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    pub fn line(&self, offset: usize) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    pub fn line_start(&self, line: u32) -> usize {
        self.starts.get(line.saturating_sub(1) as usize).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<&str> {
        tokenize(src).iter().map(|t| &src[t.start..t.end]).collect()
    }

    #[test]
    fn skips_comments_and_keeps_literals_whole() {
        let src = "// c\nconst a = 'x // y'; /* z */ b(`t ${ {a: 1}.a } u`) / 2;";
        assert_eq!(
            texts(src),
            vec!["const", "a", "=", "'x // y'", ";", "b", "(", "`t ${ {a: 1}.a } u`", ")", "/", "2", ";"]
        );
    }

    #[test]
    fn detects_regex_literals() {
        let src = "x = /ab+c\\//gi.test(y); z = a / b / c;";
        let t = texts(src);
        assert_eq!(t[2], "/ab+c\\//gi");
        assert!(t.contains(&"/"));
    }

    #[test]
    fn line_index() {
        let li = LineIndex::new("a\nb\nc");
        assert_eq!(li.line(0), 1);
        assert_eq!(li.line(2), 2);
        assert_eq!(li.line(4), 3);
    }
}
