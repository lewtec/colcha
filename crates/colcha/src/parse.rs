use std::collections::{BTreeMap, BTreeSet};

use crate::emit::{EmitKind, Emitter, Expr};
use crate::error::Error;
use crate::fact::Fact;
use crate::path::Path;
use crate::schema::{Constraint, Node, NodeKind, Schema, SchemaId};
use crate::stamp::Stamp;
use crate::value::{Float, Value};

const MAX_INPUT: usize = 1024 * 1024;
const MAX_DEPTH: usize = 32;

/// Parse DSL text into facts for `T`.
pub fn parse<T: Schema>(source: &str, stamp: Stamp) -> Result<Vec<Fact>, Error> {
    if source.len() > MAX_INPUT {
        return Err(Error::parse(
            1,
            1,
            format!("input is larger than {MAX_INPUT} bytes"),
        ));
    }
    let root = T::schema();
    let schema = SchemaId::new(T::NAME, T::VERSION);
    let mut parser = Parser {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        line: 1,
        column: 1,
        root: &root,
        emitter: Emitter::new(&root, schema, stamp),
    };
    let mut declared = BTreeSet::new();
    parser.parse_config(&Path::root(), 0, &mut declared)?;
    parser.skip_trivia();
    if parser.pos < parser.bytes.len() {
        return Err(parser.error("unexpected trailing text"));
    }
    Ok(parser.emitter.into_facts())
}

struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
    root: &'a Node,
    emitter: Emitter<'a>,
}

impl<'a> Parser<'a> {
    fn parse_config(
        &mut self,
        base: &Path,
        depth: usize,
        declared: &mut BTreeSet<Path>,
    ) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("nesting is deeper than {MAX_DEPTH}")));
        }
        loop {
            self.skip_trivia();
            if self.pos >= self.bytes.len() || self.peek_char() == Some('}') {
                return Ok(());
            }
            self.parse_field(base, depth, declared)?;
        }
    }

    fn parse_field(
        &mut self,
        base: &Path,
        depth: usize,
        declared: &mut BTreeSet<Path>,
    ) -> Result<(), Error> {
        let start = (self.line, self.column);
        let segments = self.parse_path_segments()?;
        let mut all = base.segments().to_vec();
        all.extend(segments);
        let path = Path::from_segments(all)
            .map_err(|error| Error::parse(start.0, start.1, error.to_string()))?;
        let node = self
            .root
            .lookup(&path)
            .map_err(|reason| Error::path(path.to_string(), reason))?;
        if !declared.insert(path.clone()) {
            return Err(Error::path(
                path.to_string(),
                "field declared twice in the same block",
            ));
        }

        self.skip_trivia();
        if self.eat_char(':') {
            let expr = self.parse_or(&path, node, depth)?;
            self.emitter.emit_expr(&path, node, expr)?;
            return Ok(());
        }
        if self.eat_char('{') {
            if matches!(node.kind, NodeKind::Raw | NodeKind::Union) {
                let value = self.parse_object_body(depth)?;
                self.emitter
                    .emit_value(&path, node, value, EmitKind::Candidate)?;
                return Ok(());
            }
            if node.kind == NodeKind::Leaf {
                return Err(Error::path(
                    path.to_string(),
                    "a scalar field expects a value",
                ));
            }
            let mut inner = BTreeSet::new();
            self.parse_config(&path, depth + 1, &mut inner)?;
            self.expect_char('}')?;
            return Ok(());
        }
        Err(self.error("expected ':' or '{' after a field path"))
    }

    fn parse_or(&mut self, path: &Path, node: &Node, depth: usize) -> Result<Expr, Error> {
        let mut parts = vec![self.parse_and(path, node, depth)?];
        while self.eat_char('|') {
            parts.push(self.parse_and(path, node, depth)?);
        }
        if parts.len() == 1 {
            return Ok(parts.into_iter().next().expect("one part"));
        }
        if parts.iter().all(Expr::is_pure_values) {
            let mut values = BTreeSet::new();
            let mut defaults = Vec::new();
            for part in parts {
                values.extend(part.candidates);
                values.extend(part.defaults.iter().cloned());
                defaults.extend(part.defaults);
            }
            return Ok(Expr {
                constraints: vec![Constraint::OneOf(values.into_iter().collect())],
                defaults,
                ..Expr::default()
            });
        }
        if parts.iter().all(Expr::is_pure_natives) {
            let mut names = BTreeSet::new();
            for part in parts {
                names.extend(part.natives);
            }
            return Ok(Expr {
                constraints: vec![Constraint::NativeOneOf(names.into_iter().collect())],
                ..Expr::default()
            });
        }
        Err(self.error("disjunction mixes values, constraints and natives"))
    }

    fn parse_and(&mut self, path: &Path, node: &Node, depth: usize) -> Result<Expr, Error> {
        let mut expr = self.parse_term(path, node, depth)?;
        while self.eat_char('&') {
            expr = expr.merge(self.parse_term(path, node, depth)?);
        }
        Ok(expr)
    }

    fn parse_term(&mut self, path: &Path, node: &Node, depth: usize) -> Result<Expr, Error> {
        self.skip_trivia();
        if self.eat_char('*') {
            let mut inner = self.parse_term(path, node, depth)?;
            if !inner.constraints.is_empty() || !inner.natives.is_empty() {
                return Err(self.error("a default must be a concrete value"));
            }
            inner.defaults.extend(inner.candidates);
            inner.candidates = Vec::new();
            if inner.defaults.is_empty() {
                return Err(self.error("a default needs a value"));
            }
            return Ok(inner);
        }

        if self.eat_str(">=") {
            let value = self.parse_number()?;
            return Ok(Expr {
                constraints: vec![Constraint::Min(value)],
                ..Expr::default()
            });
        }
        if self.eat_str("<=") {
            let value = self.parse_number()?;
            return Ok(Expr {
                constraints: vec![Constraint::Max(value)],
                ..Expr::default()
            });
        }
        if self.eat_str(">") {
            let value = self.parse_number()?;
            return Ok(Expr {
                constraints: vec![Constraint::Greater(value)],
                ..Expr::default()
            });
        }
        if self.eat_str("<") {
            let value = self.parse_number()?;
            return Ok(Expr {
                constraints: vec![Constraint::Less(value)],
                ..Expr::default()
            });
        }
        if self.eat_str("=~") {
            let pattern = self.parse_string_literal()?;
            regex::Regex::new(&pattern)
                .map_err(|error| self.error(format!("invalid regex: {error}")))?;
            return Ok(Expr {
                constraints: vec![Constraint::Regex(pattern)],
                ..Expr::default()
            });
        }

        if self.starts_with_ident("_core") {
            return self.parse_native(path, depth);
        }

        if let Some(keyword) = self.peek_keyword() {
            match keyword.as_str() {
                "float" | "int" | "string" | "bool" => {
                    self.consume_ident();
                    return Ok(Expr {
                        constraints: vec![Constraint::Type(keyword)],
                        ..Expr::default()
                    });
                }
                _ => {}
            }
        }

        if self.peek_char() == Some('{') {
            if matches!(node.kind, NodeKind::Raw | NodeKind::Union) {
                let value = self.parse_object(depth)?;
                return Ok(Expr {
                    candidates: vec![value],
                    ..Expr::default()
                });
            }
            let mut inner = BTreeSet::new();
            self.expect_char('{')?;
            self.parse_config(path, depth + 1, &mut inner)?;
            self.expect_char('}')?;
            return Ok(Expr::default());
        }

        let value = self.parse_value(depth)?;
        Ok(Expr {
            candidates: vec![value],
            ..Expr::default()
        })
    }

    fn parse_native(&mut self, path: &Path, depth: usize) -> Result<Expr, Error> {
        self.consume_ident();
        self.expect_char('.')?;
        self.skip_trivia();
        let name = self.parse_ident_any()?;
        self.skip_trivia();
        if self.eat_char('{') {
            let mut inner = BTreeSet::new();
            self.parse_config(path, depth + 1, &mut inner)?;
            self.expect_char('}')?;
        }
        Ok(Expr {
            candidates: vec![Value::Native {
                name: name.clone(),
                args: Box::new(Value::Map(BTreeMap::new())),
            }],
            natives: vec![name],
            ..Expr::default()
        })
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value, Error> {
        self.skip_trivia();
        let Some(next) = self.peek_char() else {
            return Err(self.error("expected a value"));
        };
        match next {
            '"' => {
                let text = self.parse_string_literal()?;
                Ok(Value::String(text))
            }
            '[' => self.parse_list(depth),
            '{' => self.parse_object(depth),
            '-' | '0'..='9' => self.parse_number(),
            _ => {
                let keyword = self
                    .peek_keyword()
                    .ok_or_else(|| self.error("expected a value"))?;
                match keyword.as_str() {
                    "true" => {
                        self.consume_ident();
                        Ok(Value::Bool(true))
                    }
                    "false" => {
                        self.consume_ident();
                        Ok(Value::Bool(false))
                    }
                    "null" => {
                        self.consume_ident();
                        Ok(Value::Null)
                    }
                    _ => Err(self.error("expected a concrete value")),
                }
            }
        }
    }

    fn parse_list(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("nesting is deeper than {MAX_DEPTH}")));
        }
        self.expect_char('[')?;
        let mut values = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat_char(']') {
                break;
            }
            values.push(self.parse_value(depth + 1)?);
            self.skip_trivia();
            if self.eat_char(',') {
                continue;
            }
            self.expect_char(']')?;
            break;
        }
        Ok(Value::List(values))
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value, Error> {
        self.expect_char('{')?;
        self.parse_object_body(depth)
    }

    fn parse_object_body(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("nesting is deeper than {MAX_DEPTH}")));
        }
        let mut map = BTreeMap::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            let key = self.parse_object_key()?;
            self.expect_char(':')?;
            let value = self.parse_value(depth + 1)?;
            if map.insert(key.clone(), value).is_some() {
                return Err(self.error(format!("duplicate key '{key}'")));
            }
            self.skip_trivia();
            let _ = self.eat_char(',');
        }
        Ok(Value::Map(map))
    }

    fn parse_object_key(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        if self.peek_char() == Some('"') {
            return self.parse_string_literal();
        }
        self.parse_ident_any()
    }

    fn parse_path_segments(&mut self) -> Result<Vec<String>, Error> {
        let mut segments = vec![self.parse_path_segment()?];
        loop {
            self.skip_trivia();
            if self.peek_char() != Some('.') {
                break;
            }
            self.bump_char();
            segments.push(self.parse_path_segment()?);
        }
        Ok(segments)
    }

    fn parse_path_segment(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        if self.peek_char() == Some('"') {
            return self.parse_string_literal();
        }
        let start = self.pos;
        let Some(first) = self.peek_char() else {
            return Err(self.error("expected a path segment"));
        };
        if !(first.is_ascii_alphanumeric() || first == '_') {
            return Err(self.error("expected a path segment"));
        }
        self.bump_char();
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' {
                self.bump_char();
            } else {
                break;
            }
        }
        Ok(self.source[start..self.pos].to_string())
    }

    fn parse_number(&mut self) -> Result<Value, Error> {
        self.skip_trivia();
        let start = self.pos;
        if self.peek_byte() == Some(b'-') {
            self.bump_char();
        }
        let mut has_digits = false;
        let mut has_dot = false;
        while let Some(byte) = self.peek_byte() {
            match byte {
                b'0'..=b'9' => {
                    has_digits = true;
                    self.bump_char();
                }
                b'.' if !has_dot => {
                    has_dot = true;
                    self.bump_char();
                }
                b'e' | b'E' => {
                    self.bump_char();
                    if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                        self.bump_char();
                    }
                }
                _ => break,
            }
        }
        if !has_digits {
            return Err(self.error("expected a number"));
        }
        let text = &self.source[start..self.pos];
        if has_dot || text.contains('e') || text.contains('E') {
            let value: f64 = text
                .parse()
                .map_err(|_| self.error(format!("invalid number '{text}'")))?;
            let float = Float::new(value)
                .ok_or_else(|| self.error("NaN is not a valid configuration value"))?;
            Ok(Value::Float(float))
        } else {
            let value: i64 = text
                .parse()
                .map_err(|_| self.error(format!("invalid integer '{text}'")))?;
            Ok(Value::Int(value))
        }
    }

    fn parse_string_literal(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        self.expect_char('"')?;
        let mut output = String::new();
        loop {
            let Some(ch) = self.bump_char() else {
                return Err(self.error("unterminated string"));
            };
            match ch {
                '"' => return Ok(output),
                '\\' => {
                    let Some(escape) = self.bump_char() else {
                        return Err(self.error("unterminated escape"));
                    };
                    match escape {
                        '"' => output.push('"'),
                        '\\' => output.push('\\'),
                        '/' => output.push('/'),
                        'b' => output.push('\u{0008}'),
                        'f' => output.push('\u{000c}'),
                        'n' => output.push('\n'),
                        'r' => output.push('\r'),
                        't' => output.push('\t'),
                        'u' => output.push(self.parse_unicode_escape()?),
                        other => {
                            return Err(self.error(format!("invalid escape '\\{other}'")));
                        }
                    }
                }
                '\n' => return Err(self.error("newline inside a string")),
                other => output.push(other),
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, Error> {
        let braced = self.peek_char() == Some('{');
        if braced {
            self.bump_char();
        }
        let limit = if braced { 6 } else { 4 };
        let mut digits = String::new();
        while digits.len() < limit {
            let Some(ch) = self.peek_char() else {
                return Err(self.error("short unicode escape"));
            };
            if braced && ch == '}' {
                break;
            }
            if !ch.is_ascii_hexdigit() {
                return Err(self.error("invalid unicode escape"));
            }
            self.bump_char();
            digits.push(ch);
        }
        if digits.is_empty() {
            return Err(self.error("invalid unicode escape"));
        }
        if braced {
            if self.peek_char() != Some('}') {
                return Err(self.error("unterminated unicode escape"));
            }
            self.bump_char();
        }
        let code =
            u32::from_str_radix(&digits, 16).map_err(|_| self.error("invalid unicode escape"))?;
        char::from_u32(code).ok_or_else(|| self.error("invalid unicode escape"))
    }

    fn parse_ident_any(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                self.bump_char();
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.error("expected an identifier"));
        }
        Ok(self.source[start..self.pos].to_string())
    }

    fn peek_keyword(&mut self) -> Option<String> {
        self.skip_trivia();
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len()
            && (self.bytes[end].is_ascii_alphanumeric() || self.bytes[end] == b'_')
        {
            end += 1;
        }
        if end == start {
            None
        } else {
            Some(self.source[start..end].to_string())
        }
    }

    fn consume_ident(&mut self) {
        self.skip_trivia();
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                self.bump_char();
            } else {
                break;
            }
        }
    }

    fn starts_with_ident(&mut self, wanted: &str) -> bool {
        self.skip_trivia();
        let rest = &self.source[self.pos..];
        rest.starts_with(wanted)
            && rest[wanted.len()..]
                .chars()
                .next()
                .map(|ch| ch == '.' || ch == '{' || ch.is_whitespace())
                .unwrap_or(true)
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek_char() {
                Some(ch) if ch.is_whitespace() => {
                    self.bump_char();
                }
                Some('/') if self.bytes.get(self.pos + 1) == Some(&b'/') => {
                    while let Some(ch) = self.peek_char() {
                        if ch == '\n' {
                            break;
                        }
                        self.bump_char();
                    }
                }
                _ => break,
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn eat_char(&mut self, wanted: char) -> bool {
        self.skip_trivia();
        if self.peek_char() == Some(wanted) {
            self.bump_char();
            true
        } else {
            false
        }
    }

    fn expect_char(&mut self, wanted: char) -> Result<(), Error> {
        if self.eat_char(wanted) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{wanted}'")))
        }
    }

    fn eat_str(&mut self, wanted: &str) -> bool {
        self.skip_trivia();
        if self.source[self.pos..].starts_with(wanted) {
            for _ in wanted.chars() {
                self.bump_char();
            }
            true
        } else {
            false
        }
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error::parse(self.line, self.column, message)
    }
}
