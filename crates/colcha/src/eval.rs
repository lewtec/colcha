//! Authored-document evaluation.
//!
//! `evaluate` parses references, interpolation, conditionals, comprehensions and
//! non-literal defaults, resolves them against the schema and a [`Context`],
//! and emits concrete [`Fact`] values. The fold and materialization stages never
//! see authored syntax.

use std::collections::{BTreeMap, BTreeSet};

use crate::emit::{Emitter, Expr};
use crate::error::Error;
use crate::fact::Fact;
use crate::path::Path;
use crate::schema::{Constraint, Node, NodeKind, Schema, SchemaId};
use crate::stamp::Stamp;
use crate::value::{Float, Value};

const MAX_INPUT: usize = 1024 * 1024;
const MAX_DEPTH: usize = 32;

/// Values supplied by the host for references that do not live in the document.
///
/// Document fields win over context values. A context path may be read as a
/// whole value or as the root of a nested map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    values: BTreeMap<Path, Value>,
}

impl Context {
    pub fn new() -> Context {
        Context::default()
    }

    /// Return a new context with one more value. The receiver stays unchanged.
    ///
    /// Repeating a path with an equal value is idempotent. A different value at
    /// the same path is `Error::Eval`.
    pub fn with(&self, path: impl AsRef<str>, value: Value) -> Result<Context, Error> {
        let path = Path::parse(path.as_ref())?;
        self.merge(&Context {
            values: BTreeMap::from([(path, value)]),
        })
    }

    /// Combine two immutable contexts into a new one.
    ///
    /// Equal values at the same path are idempotent. Different values at the
    /// same path are `Error::Eval`.
    pub fn merge(&self, other: &Context) -> Result<Context, Error> {
        let mut values = self.values.clone();
        for (path, value) in &other.values {
            match values.get(path) {
                Some(existing) if existing != value => {
                    return Err(Error::eval(
                        path.to_string(),
                        "context values conflict at the same path",
                    ));
                }
                _ => {
                    values.insert(path.clone(), value.clone());
                }
            }
        }
        Ok(Context { values })
    }

    pub fn get(&self, path: &Path) -> Option<&Value> {
        self.values.get(path)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Parse concrete text into facts for `T`.
///
/// The concrete mode accepts values, constraints, defaults, natives, object and
/// list literals, and block shorthand. References, interpolation, conditionals,
/// comprehensions and templated keys are rejected as parse errors.
pub fn parse<T: Schema>(source: &str, stamp: Stamp) -> Result<Vec<Fact>, Error> {
    run::<T>(source, &Context::new(), stamp, true)
}

/// Evaluate authored text into concrete facts for `T`.
///
/// All references, conditionals, comprehensions, interpolation and non-literal
/// defaults are resolved before a fact is produced. `fold` and `Materialize`
/// only see the returned concrete values.
pub fn evaluate<T: Schema>(
    source: &str,
    context: &Context,
    stamp: Stamp,
) -> Result<Vec<Fact>, Error> {
    run::<T>(source, context, stamp, false)
}

fn run<T: Schema>(
    source: &str,
    context: &Context,
    stamp: Stamp,
    concrete: bool,
) -> Result<Vec<Fact>, Error> {
    if source.len() > MAX_INPUT {
        return Err(Error::parse(
            1,
            1,
            format!("input is larger than {MAX_INPUT} bytes"),
        ));
    }
    let root = T::schema();
    let schema = SchemaId::new(T::NAME, T::VERSION);
    let mut parser = if concrete {
        Parser::concrete(source)
    } else {
        Parser::new(source)
    };
    let entries = parser.parse_document()?;
    let mut collector = if concrete {
        Collector::concrete(&root)
    } else {
        Collector::new(&root)
    };
    collector.collect_block(&Path::root(), &entries, &[], &Locals::default())?;
    collector.expand_tasks(context)?;
    collector.emit(context, schema, stamp)
}

#[derive(Clone, Debug)]
struct Template {
    pieces: Vec<Piece>,
}

#[derive(Clone, Debug)]
enum Piece {
    Literal(String),
    Reference(Vec<String>),
}

#[derive(Clone, Debug)]
enum Segment {
    Literal(String),
    Template(Template),
}

#[derive(Clone, Debug)]
enum ValueAst {
    Null,
    Bool(bool),
    Int(i64),
    Float(Float),
    String(String),
    Template(Template),
    List(Vec<ExprAst>),
    Map(BTreeMap<String, ExprAst>),
    Reference(Vec<String>),
}

#[derive(Clone, Debug)]
enum ExprAst {
    Value(ValueAst),
    Constraint(Constraint),
    Native {
        name: String,
        args: Option<ValueAst>,
    },
    Default(Box<ExprAst>),
    And(Vec<ExprAst>),
    Or(Vec<ExprAst>),
}

#[derive(Clone, Debug)]
enum ConditionAst {
    Truthy(ExprAst),
    Equal(ExprAst, ExprAst),
    NotEqual(ExprAst, ExprAst),
}

#[derive(Clone, Debug)]
struct Entry {
    kind: EntryKind,
}

#[derive(Clone, Debug)]
enum EntryKind {
    Field {
        segments: Vec<Segment>,
        value: FieldValue,
    },
    Conditional {
        condition: ConditionAst,
        then_entries: Vec<Entry>,
        else_entries: Vec<Entry>,
    },
    Comprehension {
        key: Option<String>,
        value: String,
        source: ValueAst,
        body: Vec<Entry>,
    },
}

#[derive(Clone, Debug)]
enum FieldValue {
    Expr(ExprAst),
    Block(Vec<Entry>),
}

#[derive(Clone, Debug, Default)]
struct Locals(BTreeMap<String, Value>);

impl Locals {
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    fn insert(&mut self, name: String, value: Value) {
        self.0.insert(name, value);
    }
}

struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
    concrete: bool,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Parser<'a> {
        Parser {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            column: 1,
            concrete: false,
        }
    }

    fn concrete(source: &'a str) -> Parser<'a> {
        Parser {
            concrete: true,
            ..Parser::new(source)
        }
    }

    fn parse_document(&mut self) -> Result<Vec<Entry>, Error> {
        let entries = self.parse_entries(0)?;
        self.skip_trivia();
        if self.pos < self.bytes.len() {
            return Err(self.error("unexpected trailing text"));
        }
        Ok(entries)
    }

    fn parse_entries(&mut self, depth: usize) -> Result<Vec<Entry>, Error> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("nesting is deeper than {MAX_DEPTH}")));
        }
        let mut entries = Vec::new();
        loop {
            self.skip_trivia();
            if self.pos >= self.bytes.len() || self.peek_char() == Some('}') {
                return Ok(entries);
            }
            entries.push(self.parse_entry(depth)?);
            self.skip_trivia();
            let _ = self.eat_char(',');
        }
    }

    fn parse_entry(&mut self, depth: usize) -> Result<Entry, Error> {
        self.skip_trivia();
        if self.starts_with_ident("if") {
            if self.concrete {
                return Err(self.error("conditionals are not allowed in concrete input"));
            }
            return self.parse_conditional(depth);
        }
        if self.starts_with_ident("for") {
            if self.concrete {
                return Err(self.error("comprehensions are not allowed in concrete input"));
            }
            return self.parse_comprehension(depth);
        }
        self.parse_field(depth)
    }

    fn parse_field(&mut self, depth: usize) -> Result<Entry, Error> {
        let segments = self.parse_key_segments()?;
        self.skip_trivia();
        if self.eat_char(':') {
            let value = self.parse_field_value(depth)?;
            return Ok(Entry {
                kind: EntryKind::Field { segments, value },
            });
        }
        if self.eat_char('{') {
            let body = self.parse_entries(depth + 1)?;
            self.expect_char('}')?;
            return Ok(Entry {
                kind: EntryKind::Field {
                    segments,
                    value: FieldValue::Block(body),
                },
            });
        }
        Err(self.error("expected ':' or '{' after a field path"))
    }

    fn parse_field_value(&mut self, depth: usize) -> Result<FieldValue, Error> {
        self.skip_trivia();
        if self.peek_char() == Some('{') {
            self.expect_char('{')?;
            let body = self.parse_entries(depth + 1)?;
            self.expect_char('}')?;
            return Ok(FieldValue::Block(body));
        }

        if let Some(nested) = self.try_parse_nested_field(depth)? {
            return Ok(FieldValue::Block(vec![nested]));
        }

        Ok(FieldValue::Expr(self.parse_expr(depth)?))
    }

    /// Recognize CUE-style chains such as `file: home: { ... }` and
    /// `values: "a.b": 1`.
    fn try_parse_nested_field(&mut self, depth: usize) -> Result<Option<Entry>, Error> {
        let snapshot = (self.pos, self.line, self.column);
        let Ok(segments) = self.parse_key_segments() else {
            self.restore(snapshot);
            return Ok(None);
        };
        if segments.is_empty() {
            self.restore(snapshot);
            return Ok(None);
        }
        if let Some(Segment::Literal(first)) = segments.first() {
            if first == "_core" {
                self.restore(snapshot);
                return Ok(None);
            }
        }
        self.skip_trivia();
        let value = if self.eat_char(':') {
            self.parse_field_value(depth + 1)?
        } else if self.eat_char('{') {
            let body = self.parse_entries(depth + 1)?;
            self.expect_char('}')?;
            FieldValue::Block(body)
        } else {
            self.restore(snapshot);
            return Ok(None);
        };
        Ok(Some(Entry {
            kind: EntryKind::Field { segments, value },
        }))
    }

    fn restore(&mut self, snapshot: (usize, usize, usize)) {
        self.pos = snapshot.0;
        self.line = snapshot.1;
        self.column = snapshot.2;
    }

    fn parse_conditional(&mut self, depth: usize) -> Result<Entry, Error> {
        self.consume_ident();
        let condition = self.parse_condition(depth)?;
        self.expect_char('{')?;
        let then_entries = self.parse_entries(depth + 1)?;
        self.expect_char('}')?;
        self.skip_trivia();
        let else_entries = if self.starts_with_ident("else") {
            self.consume_ident();
            self.skip_trivia();
            if self.starts_with_ident("if") {
                vec![self.parse_conditional(depth)?]
            } else {
                self.expect_char('{')?;
                let entries = self.parse_entries(depth + 1)?;
                self.expect_char('}')?;
                entries
            }
        } else {
            Vec::new()
        };
        Ok(Entry {
            kind: EntryKind::Conditional {
                condition,
                then_entries,
                else_entries,
            },
        })
    }

    fn parse_condition(&mut self, depth: usize) -> Result<ConditionAst, Error> {
        let left = self.parse_condition_operand(depth)?;
        if self.eat_str("==") {
            let right = self.parse_condition_operand(depth)?;
            return Ok(ConditionAst::Equal(left, right));
        }
        if self.eat_str("!=") {
            let right = self.parse_condition_operand(depth)?;
            return Ok(ConditionAst::NotEqual(left, right));
        }
        Ok(ConditionAst::Truthy(left))
    }

    fn parse_condition_operand(&mut self, depth: usize) -> Result<ExprAst, Error> {
        Ok(ExprAst::Value(self.parse_value(depth)?))
    }

    fn parse_comprehension(&mut self, depth: usize) -> Result<Entry, Error> {
        self.consume_ident();
        let first = self.parse_ident_any()?;
        self.skip_trivia();
        let (key, value) = if self.eat_char(',') {
            let second = self.parse_ident_any()?;
            (Some(first), second)
        } else {
            (None, first)
        };
        self.skip_trivia();
        if !self.starts_with_ident("in") {
            return Err(self.error("expected 'in' after a comprehension binding"));
        }
        self.consume_ident();
        let source = self.parse_value(depth)?;
        self.expect_char('{')?;
        let body = self.parse_entries(depth + 1)?;
        self.expect_char('}')?;
        Ok(Entry {
            kind: EntryKind::Comprehension {
                key,
                value,
                source,
                body,
            },
        })
    }

    fn parse_expr(&mut self, depth: usize) -> Result<ExprAst, Error> {
        self.parse_or(depth)
    }

    fn parse_or(&mut self, depth: usize) -> Result<ExprAst, Error> {
        let mut parts = vec![self.parse_and(depth)?];
        while self.eat_char('|') {
            parts.push(self.parse_and(depth)?);
        }
        if parts.len() == 1 {
            Ok(parts.pop().expect("one part"))
        } else {
            Ok(ExprAst::Or(parts))
        }
    }

    fn parse_and(&mut self, depth: usize) -> Result<ExprAst, Error> {
        let mut parts = vec![self.parse_term(depth)?];
        while self.eat_char('&') {
            parts.push(self.parse_term(depth)?);
        }
        if parts.len() == 1 {
            Ok(parts.pop().expect("one part"))
        } else {
            Ok(ExprAst::And(parts))
        }
    }

    fn parse_term(&mut self, depth: usize) -> Result<ExprAst, Error> {
        self.skip_trivia();
        if self.eat_char('*') {
            let inner = self.parse_term(depth)?;
            return Ok(ExprAst::Default(Box::new(inner)));
        }

        if self.eat_str(">=") {
            return Ok(ExprAst::Constraint(Constraint::Min(self.parse_number()?)));
        }
        if self.eat_str("<=") {
            return Ok(ExprAst::Constraint(Constraint::Max(self.parse_number()?)));
        }
        if self.eat_str(">") {
            return Ok(ExprAst::Constraint(Constraint::Greater(
                self.parse_number()?,
            )));
        }
        if self.eat_str("<") {
            return Ok(ExprAst::Constraint(Constraint::Less(self.parse_number()?)));
        }
        if self.eat_str("=~") {
            let pattern = self.parse_plain_string()?;
            regex::Regex::new(&pattern)
                .map_err(|error| self.error(format!("invalid regex: {error}")))?;
            return Ok(ExprAst::Constraint(Constraint::Regex(pattern)));
        }

        if self.starts_with_ident("_core") {
            return self.parse_native();
        }

        if let Some(keyword) = self.peek_keyword() {
            match keyword.as_str() {
                "float" | "int" | "string" | "bool" => {
                    self.consume_ident();
                    return Ok(ExprAst::Constraint(Constraint::Type(keyword)));
                }
                _ => {}
            }
        }

        Ok(ExprAst::Value(self.parse_value(depth)?))
    }

    fn parse_native(&mut self) -> Result<ExprAst, Error> {
        self.consume_ident();
        self.expect_char('.')?;
        let name = self.parse_ident_any()?;
        self.skip_trivia();
        let args = if self.peek_char() == Some('{') {
            Some(self.parse_object_value(0)?)
        } else {
            None
        };
        Ok(ExprAst::Native { name, args })
    }

    fn parse_value(&mut self, depth: usize) -> Result<ValueAst, Error> {
        self.skip_trivia();
        let Some(next) = self.peek_char() else {
            return Err(self.error("expected a value"));
        };
        match next {
            '"' => match self.parse_string_value()? {
                StringValue::Literal(text) => Ok(ValueAst::String(text)),
                StringValue::Template(_) if self.concrete => {
                    Err(self.error("interpolation is not allowed in concrete input"))
                }
                StringValue::Template(template) => Ok(ValueAst::Template(template)),
            },
            '[' => self.parse_list_value(depth),
            '{' => self.parse_object_value(depth),
            '-' | '0'..='9' => self.parse_number().map(value_from_number),
            _ => {
                let keyword = self
                    .peek_keyword()
                    .ok_or_else(|| self.error("expected a value"))?;
                match keyword.as_str() {
                    "true" => {
                        self.consume_ident();
                        Ok(ValueAst::Bool(true))
                    }
                    "false" => {
                        self.consume_ident();
                        Ok(ValueAst::Bool(false))
                    }
                    "null" => {
                        self.consume_ident();
                        Ok(ValueAst::Null)
                    }
                    _ => {
                        if self.concrete {
                            return Err(self.error("expected a concrete value"));
                        }
                        let path = self.parse_reference_segments()?;
                        Ok(ValueAst::Reference(path))
                    }
                }
            }
        }
    }

    fn parse_list_value(&mut self, depth: usize) -> Result<ValueAst, Error> {
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
            values.push(self.parse_expr(depth + 1)?);
            self.skip_trivia();
            if self.eat_char(',') {
                continue;
            }
            self.expect_char(']')?;
            break;
        }
        Ok(ValueAst::List(values))
    }

    fn parse_object_value(&mut self, depth: usize) -> Result<ValueAst, Error> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("nesting is deeper than {MAX_DEPTH}")));
        }
        self.expect_char('{')?;
        let mut map = BTreeMap::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            let key = self.parse_object_key()?;
            self.expect_char(':')?;
            let value = self.parse_expr(depth + 1)?;
            if map.insert(key.clone(), value).is_some() {
                return Err(self.error(format!("duplicate key '{key}'")));
            }
            self.skip_trivia();
            let _ = self.eat_char(',');
        }
        Ok(ValueAst::Map(map))
    }

    fn parse_object_key(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        if self.peek_char() == Some('"') {
            return self.parse_plain_string();
        }
        self.parse_ident_any()
    }

    fn parse_key_segments(&mut self) -> Result<Vec<Segment>, Error> {
        let mut segments = vec![self.parse_key_segment()?];
        loop {
            self.skip_trivia();
            if self.peek_char() != Some('.') {
                break;
            }
            self.bump_char();
            segments.push(self.parse_key_segment()?);
        }
        Ok(segments)
    }

    fn parse_key_segment(&mut self) -> Result<Segment, Error> {
        self.skip_trivia();
        if self.peek_char() == Some('"') {
            return match self.parse_string_value()? {
                StringValue::Literal(text) => Ok(Segment::Literal(text)),
                StringValue::Template(_) if self.concrete => {
                    Err(self.error("interpolation is not allowed in a concrete key"))
                }
                StringValue::Template(template) => Ok(Segment::Template(template)),
            };
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
        Ok(Segment::Literal(self.source[start..self.pos].to_string()))
    }

    fn parse_reference_segments(&mut self) -> Result<Vec<String>, Error> {
        let mut segments = vec![self.parse_reference_segment()?];
        loop {
            self.skip_trivia();
            if self.peek_char() != Some('.') {
                break;
            }
            self.bump_char();
            segments.push(self.parse_reference_segment()?);
        }
        Ok(segments)
    }

    fn parse_reference_segment(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        if self.peek_char() == Some('"') {
            return self.parse_plain_string();
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

    fn parse_plain_string(&mut self) -> Result<String, Error> {
        match self.parse_string_value()? {
            StringValue::Literal(text) => Ok(text),
            StringValue::Template(_) => Err(self.error("interpolation is not allowed here")),
        }
    }

    fn parse_string_value(&mut self) -> Result<StringValue, Error> {
        self.skip_trivia();
        self.expect_char('"')?;
        let mut pieces = Vec::new();
        let mut literal = String::new();
        loop {
            let Some(ch) = self.bump_char() else {
                return Err(self.error("unterminated string"));
            };
            match ch {
                '"' => {
                    if pieces.is_empty() {
                        return Ok(StringValue::Literal(literal));
                    }
                    if !literal.is_empty() {
                        pieces.push(Piece::Literal(literal));
                    }
                    return Ok(StringValue::Template(Template { pieces }));
                }
                '\\' => {
                    let Some(escape) = self.bump_char() else {
                        return Err(self.error("unterminated escape"));
                    };
                    if escape == '(' {
                        if !literal.is_empty() {
                            pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                        }
                        let reference = self.parse_reference_segments()?;
                        self.skip_trivia();
                        self.expect_char(')')?;
                        pieces.push(Piece::Reference(reference));
                        continue;
                    }
                    match escape {
                        '"' => literal.push('"'),
                        '\\' => literal.push('\\'),
                        '/' => literal.push('/'),
                        'b' => literal.push('\u{0008}'),
                        'f' => literal.push('\u{000c}'),
                        'n' => literal.push('\n'),
                        'r' => literal.push('\r'),
                        't' => literal.push('\t'),
                        'u' => literal.push(self.parse_unicode_escape()?),
                        other => {
                            return Err(self.error(format!("invalid escape '\\{other}'")));
                        }
                    }
                }
                '\n' => return Err(self.error("newline inside a string")),
                other => literal.push(other),
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

enum StringValue {
    Literal(String),
    Template(Template),
}

fn value_from_number(value: Value) -> ValueAst {
    match value {
        Value::Int(value) => ValueAst::Int(value),
        Value::Float(value) => ValueAst::Float(value),
        _ => unreachable!("parse_number returns a number"),
    }
}

#[derive(Clone)]
struct Guard {
    condition: ConditionAst,
    locals: Locals,
    positive: bool,
}

struct Decl {
    path: Path,
    value: DeclValue,
    guards: Vec<Guard>,
    locals: Locals,
}

enum DeclValue {
    Expr(ExprAst),
    Block(Vec<Entry>),
}

struct Task {
    base: Path,
    key: Option<String>,
    value: String,
    source: ValueAst,
    body: Vec<Entry>,
    guards: Vec<Guard>,
    locals: Locals,
}

struct Collector<'a> {
    root: &'a Node,
    decls: Vec<Decl>,
    tasks: Vec<Task>,
    concrete: bool,
}

impl<'a> Collector<'a> {
    fn new(root: &'a Node) -> Collector<'a> {
        Collector {
            root,
            decls: Vec::new(),
            tasks: Vec::new(),
            concrete: false,
        }
    }

    fn concrete(root: &'a Node) -> Collector<'a> {
        Collector {
            concrete: true,
            ..Collector::new(root)
        }
    }

    fn collect_block(
        &mut self,
        base: &Path,
        entries: &[Entry],
        guards: &[Guard],
        locals: &Locals,
    ) -> Result<(), Error> {
        let mut declared: BTreeSet<Path> = BTreeSet::new();
        for entry in entries {
            match &entry.kind {
                EntryKind::Field { segments, value } => {
                    let path = build_path(base, segments, locals)?;
                    if self.concrete && !declared.insert(path.clone()) {
                        return Err(Error::path(
                            path.to_string(),
                            "field declared twice in the same block",
                        ));
                    }
                    let node = self
                        .root
                        .lookup(&path)
                        .map_err(|reason| Error::path(path.to_string(), reason))?;
                    match value {
                        FieldValue::Expr(ast) => self.decls.push(Decl {
                            path,
                            value: DeclValue::Expr(ast.clone()),
                            guards: guards.to_vec(),
                            locals: locals.clone(),
                        }),
                        FieldValue::Block(body) => match node.kind {
                            NodeKind::Leaf => {
                                return Err(Error::path(
                                    path.to_string(),
                                    "a scalar field expects a value",
                                ));
                            }
                            NodeKind::Native => {
                                return Err(Error::path(
                                    path.to_string(),
                                    "a native field expects a native value",
                                ));
                            }
                            NodeKind::Raw | NodeKind::Union => self.decls.push(Decl {
                                path,
                                value: DeclValue::Block(body.clone()),
                                guards: guards.to_vec(),
                                locals: locals.clone(),
                            }),
                            NodeKind::Struct | NodeKind::Map => {
                                self.collect_block(&path, body, guards, locals)?;
                            }
                            NodeKind::List if node.key_field.is_some() => {
                                self.collect_block(&path, body, guards, locals)?;
                            }
                            NodeKind::List => {
                                return Err(Error::path(
                                    path.to_string(),
                                    "an atomic list expects a list value",
                                ));
                            }
                        },
                    }
                }
                EntryKind::Conditional {
                    condition,
                    then_entries,
                    else_entries,
                } => {
                    let mut then_guards = guards.to_vec();
                    then_guards.push(Guard {
                        condition: condition.clone(),
                        locals: locals.clone(),
                        positive: true,
                    });
                    self.collect_block(base, then_entries, &then_guards, locals)?;
                    let mut else_guards = guards.to_vec();
                    else_guards.push(Guard {
                        condition: condition.clone(),
                        locals: locals.clone(),
                        positive: false,
                    });
                    self.collect_block(base, else_entries, &else_guards, locals)?;
                }
                EntryKind::Comprehension {
                    key,
                    value,
                    source,
                    body,
                } => self.tasks.push(Task {
                    base: base.clone(),
                    key: key.clone(),
                    value: value.clone(),
                    source: source.clone(),
                    body: body.clone(),
                    guards: guards.to_vec(),
                    locals: locals.clone(),
                }),
            }
        }
        Ok(())
    }

    fn expand_tasks(&mut self, context: &Context) -> Result<(), Error> {
        while !self.tasks.is_empty() {
            let mut progress = false;
            let mut deferred = Vec::new();
            for task in std::mem::take(&mut self.tasks) {
                let status = {
                    let resolver = Resolver::new(self.root, &self.decls, context);
                    let mut stack = Vec::new();
                    resolver.guards_value(&task.guards, &task.base, &mut stack)?
                };
                match status {
                    Some(false) => {
                        progress = true;
                        continue;
                    }
                    None => {
                        deferred.push(task);
                        continue;
                    }
                    Some(true) => {}
                }
                let source = {
                    let resolver = Resolver::new(self.root, &self.decls, context);
                    let mut stack = Vec::new();
                    resolver.value(&task.source, &task.locals, &task.base, &mut stack)?
                };
                let Some(source) = source else {
                    deferred.push(task);
                    continue;
                };
                progress = true;
                self.expand_task(task, source)?;
            }
            if !progress {
                let task = deferred.first().expect("a task is pending");
                return Err(Error::eval(
                    task.base.to_string(),
                    "comprehension guard or source does not resolve to a concrete value",
                ));
            }
            self.tasks = deferred;
        }
        Ok(())
    }

    fn expand_task(&mut self, task: Task, source: Value) -> Result<(), Error> {
        match source {
            Value::Map(map) => {
                for (key, value) in map {
                    let mut locals = task.locals.clone();
                    if let Some(name) = &task.key {
                        locals.insert(name.clone(), Value::String(key));
                    }
                    locals.insert(task.value.clone(), value);
                    self.collect_block(&task.base, &task.body, &task.guards, &locals)?;
                }
            }
            Value::List(items) => {
                for (index, value) in items.into_iter().enumerate() {
                    let mut locals = task.locals.clone();
                    if let Some(name) = &task.key {
                        locals.insert(name.clone(), Value::Int(index as i64));
                    }
                    locals.insert(task.value.clone(), value);
                    self.collect_block(&task.base, &task.body, &task.guards, &locals)?;
                }
            }
            other => {
                return Err(Error::eval(
                    task.base.to_string(),
                    format!(
                        "a comprehension source is a map or list, found {}",
                        other.type_name()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn emit(self, context: &Context, schema: SchemaId, stamp: Stamp) -> Result<Vec<Fact>, Error> {
        let mut emitter = Emitter::new(self.root, schema, stamp);
        if self.concrete {
            for decl in &self.decls {
                Self::emit_decl(&mut emitter, self.root, &self.decls, context, decl)?;
            }
            return Ok(emitter.into_facts());
        }

        let mut active: BTreeMap<Path, &Decl> = BTreeMap::new();
        for decl in &self.decls {
            let resolver = Resolver::new(self.root, &self.decls, context);
            let mut stack = Vec::new();
            if !resolver.active(decl, &mut stack)? {
                continue;
            }
            if active.insert(decl.path.clone(), decl).is_some() {
                return Err(Error::eval(
                    decl.path.to_string(),
                    "field is declared more than once",
                ));
            }
        }

        for decl in active.values() {
            Self::emit_decl(&mut emitter, self.root, &self.decls, context, decl)?;
        }
        Ok(emitter.into_facts())
    }

    fn emit_decl(
        emitter: &mut Emitter<'_>,
        root: &Node,
        decls: &[Decl],
        context: &Context,
        decl: &Decl,
    ) -> Result<(), Error> {
        let resolver = Resolver::new(root, decls, context);
        let mut stack = Vec::new();
        let expr = resolver.expr_for(decl, &mut stack)?.ok_or_else(|| {
            Error::eval(
                decl.path.to_string(),
                "expression does not resolve to a concrete value",
            )
        })?;
        let node = root
            .lookup(&decl.path)
            .map_err(|reason| Error::path(decl.path.to_string(), reason))?;
        emitter.emit_expr(&decl.path, node, expr)
    }
}

fn build_path(base: &Path, segments: &[Segment], locals: &Locals) -> Result<Path, Error> {
    let mut path = base.clone();
    for segment in segments {
        let segment = match segment {
            Segment::Literal(text) => text.clone(),
            Segment::Template(template) => evaluate_template(template, locals, base)?,
        };
        path = path
            .child(&segment)
            .map_err(|error| Error::eval(base.to_string(), error.to_string()))?;
    }
    Ok(path)
}

fn evaluate_template(template: &Template, locals: &Locals, site: &Path) -> Result<String, Error> {
    let mut output = String::new();
    for piece in &template.pieces {
        match piece {
            Piece::Literal(text) => output.push_str(text),
            Piece::Reference(reference) => {
                let path = Path::from_segments(reference.clone())
                    .map_err(|error| Error::eval(site.to_string(), error.to_string()))?;
                let value = resolve_local(locals, &path)?.ok_or_else(|| {
                    Error::eval(
                        site.to_string(),
                        format!(
                            "a key template can only use comprehension bindings, found '{path}'"
                        ),
                    )
                })?;
                output.push_str(&scalar_text(&value, site)?);
            }
        }
    }
    Ok(output)
}

struct Resolver<'a> {
    root: &'a Node,
    decls: &'a [Decl],
    context: &'a Context,
}

impl<'a> Resolver<'a> {
    fn new(root: &'a Node, decls: &'a [Decl], context: &'a Context) -> Resolver<'a> {
        Resolver {
            root,
            decls,
            context,
        }
    }

    fn active(&self, decl: &Decl, stack: &mut Vec<Path>) -> Result<bool, Error> {
        for guard in &decl.guards {
            let value = self.condition(&guard.condition, &guard.locals, &decl.path, stack)?;
            if value != guard.positive {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn condition(
        &self,
        condition: &ConditionAst,
        locals: &Locals,
        site: &Path,
        stack: &mut Vec<Path>,
    ) -> Result<bool, Error> {
        self.condition_value(condition, locals, site, stack)?
            .ok_or_else(|| Error::eval(site.to_string(), "condition is not concrete"))
    }

    fn condition_value(
        &self,
        condition: &ConditionAst,
        locals: &Locals,
        site: &Path,
        stack: &mut Vec<Path>,
    ) -> Result<Option<bool>, Error> {
        match condition {
            ConditionAst::Truthy(expr) => {
                let Some(value) = self.expr_value(expr, locals, site, stack)? else {
                    return Ok(None);
                };
                Ok(Some(truthy(&value)))
            }
            ConditionAst::Equal(left, right) | ConditionAst::NotEqual(left, right) => {
                let Some(left) = self.expr_value(left, locals, site, stack)? else {
                    return Ok(None);
                };
                let Some(right) = self.expr_value(right, locals, site, stack)? else {
                    return Ok(None);
                };
                let equal = values_equal(&left, &right);
                Ok(Some(match condition {
                    ConditionAst::Equal(_, _) => equal,
                    _ => !equal,
                }))
            }
        }
    }

    fn guards_value(
        &self,
        guards: &[Guard],
        site: &Path,
        stack: &mut Vec<Path>,
    ) -> Result<Option<bool>, Error> {
        for guard in guards {
            let Some(value) = self.condition_value(&guard.condition, &guard.locals, site, stack)?
            else {
                return Ok(None);
            };
            if value != guard.positive {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    fn expr_for(&self, decl: &Decl, stack: &mut Vec<Path>) -> Result<Option<Expr>, Error> {
        match &decl.value {
            DeclValue::Expr(ast) => self.expr_ast(ast, &decl.locals, &decl.path, stack),
            DeclValue::Block(body) => {
                let value = self.block(body, &decl.path, &decl.locals, stack)?;
                Ok(Some(Expr {
                    candidates: vec![Value::Map(value)],
                    ..Expr::default()
                }))
            }
        }
    }

    fn block(
        &self,
        entries: &[Entry],
        base: &Path,
        locals: &Locals,
        stack: &mut Vec<Path>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        let mut map = BTreeMap::new();
        for entry in entries {
            match &entry.kind {
                EntryKind::Field { segments, value } => {
                    let path = build_path(base, segments, locals)?;
                    let relative = path.strip_prefix(base).ok_or_else(|| {
                        Error::eval(base.to_string(), "block path escaped its root")
                    })?;
                    let value = match value {
                        FieldValue::Expr(ast) => {
                            self.expr_value(ast, locals, &path, stack)?.ok_or_else(|| {
                                Error::eval(
                                    path.to_string(),
                                    "block field does not resolve to a concrete value",
                                )
                            })?
                        }
                        FieldValue::Block(body) => {
                            Value::Map(self.block(body, &path, locals, stack)?)
                        }
                    };
                    insert_nested(&mut map, relative.segments(), value, &path)?;
                }
                EntryKind::Conditional {
                    condition,
                    then_entries,
                    else_entries,
                } => {
                    let branch = if self.condition(condition, locals, base, stack)? {
                        then_entries
                    } else {
                        else_entries
                    };
                    let nested = self.block(branch, base, locals, stack)?;
                    merge_maps(&mut map, nested, base)?;
                }
                EntryKind::Comprehension {
                    key,
                    value,
                    source,
                    body,
                } => {
                    let source = self.value(source, locals, base, stack)?.ok_or_else(|| {
                        Error::eval(
                            base.to_string(),
                            "comprehension source does not resolve to a concrete value",
                        )
                    })?;
                    match source {
                        Value::Map(source) => {
                            for (source_key, source_value) in source {
                                let mut inner = locals.clone();
                                if let Some(name) = key {
                                    inner.insert(name.clone(), Value::String(source_key));
                                }
                                inner.insert(value.clone(), source_value);
                                let nested = self.block(body, base, &inner, stack)?;
                                merge_maps(&mut map, nested, base)?;
                            }
                        }
                        Value::List(items) => {
                            for (index, source_value) in items.into_iter().enumerate() {
                                let mut inner = locals.clone();
                                if let Some(name) = key {
                                    inner.insert(name.clone(), Value::Int(index as i64));
                                }
                                inner.insert(value.clone(), source_value);
                                let nested = self.block(body, base, &inner, stack)?;
                                merge_maps(&mut map, nested, base)?;
                            }
                        }
                        other => {
                            return Err(Error::eval(
                                base.to_string(),
                                format!(
                                    "a comprehension source is a map or list, found {}",
                                    other.type_name()
                                ),
                            ));
                        }
                    }
                }
            }
        }
        Ok(map)
    }

    fn expr_value(
        &self,
        ast: &ExprAst,
        locals: &Locals,
        site: &Path,
        stack: &mut Vec<Path>,
    ) -> Result<Option<Value>, Error> {
        let Some(expr) = self.expr_ast(ast, locals, site, stack)? else {
            return Ok(None);
        };
        effective(expr, site)
    }

    fn expr_ast(
        &self,
        ast: &ExprAst,
        locals: &Locals,
        site: &Path,
        stack: &mut Vec<Path>,
    ) -> Result<Option<Expr>, Error> {
        match ast {
            ExprAst::Value(value) => {
                let Some(value) = self.value(value, locals, site, stack)? else {
                    return Ok(None);
                };
                Ok(Some(Expr {
                    candidates: vec![value],
                    ..Expr::default()
                }))
            }
            ExprAst::Constraint(constraint) => Ok(Some(Expr {
                constraints: vec![constraint.clone()],
                ..Expr::default()
            })),
            ExprAst::Native { name, args } => {
                let args = match args {
                    Some(args) => match self.value(args, locals, site, stack)? {
                        Some(args) => args,
                        None => return Ok(None),
                    },
                    None => Value::Map(BTreeMap::new()),
                };
                Ok(Some(Expr {
                    candidates: vec![Value::Native {
                        name: name.clone(),
                        args: Box::new(args),
                    }],
                    natives: vec![name.clone()],
                    ..Expr::default()
                }))
            }
            ExprAst::Default(inner) => {
                let Some(inner) = self.expr_ast(inner, locals, site, stack)? else {
                    return Ok(None);
                };
                if !inner.constraints.is_empty() || !inner.natives.is_empty() {
                    return Err(Error::eval(
                        site.to_string(),
                        "a default must be a concrete value",
                    ));
                }
                if inner.candidates.is_empty() {
                    return Err(Error::eval(site.to_string(), "a default needs a value"));
                }
                Ok(Some(Expr {
                    defaults: inner.candidates,
                    ..Expr::default()
                }))
            }
            ExprAst::And(parts) => {
                let mut output = Expr::default();
                for part in parts {
                    let Some(part) = self.expr_ast(part, locals, site, stack)? else {
                        return Ok(None);
                    };
                    output = output.merge(part);
                }
                Ok(Some(output))
            }
            ExprAst::Or(parts) => {
                let mut evaluated = Vec::with_capacity(parts.len());
                for part in parts {
                    let Some(part) = self.expr_ast(part, locals, site, stack)? else {
                        return Ok(None);
                    };
                    evaluated.push(part);
                }
                Ok(Some(combine_or(evaluated, site)?))
            }
        }
    }

    fn value(
        &self,
        ast: &ValueAst,
        locals: &Locals,
        site: &Path,
        stack: &mut Vec<Path>,
    ) -> Result<Option<Value>, Error> {
        match ast {
            ValueAst::Null => Ok(Some(Value::Null)),
            ValueAst::Bool(value) => Ok(Some(Value::Bool(*value))),
            ValueAst::Int(value) => Ok(Some(Value::Int(*value))),
            ValueAst::Float(value) => Ok(Some(Value::Float(*value))),
            ValueAst::String(value) => Ok(Some(Value::String(value.clone()))),
            ValueAst::Template(template) => {
                let mut output = String::new();
                for piece in &template.pieces {
                    match piece {
                        Piece::Literal(text) => output.push_str(text),
                        Piece::Reference(reference) => {
                            let path = Path::from_segments(reference.clone()).map_err(|error| {
                                Error::eval(site.to_string(), error.to_string())
                            })?;
                            let Some(value) = self.resolve_path(&path, locals, stack)? else {
                                return Ok(None);
                            };
                            output.push_str(&scalar_text(&value, site)?);
                        }
                    }
                }
                Ok(Some(Value::String(output)))
            }
            ValueAst::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    let Some(value) = self.expr_value(item, locals, site, stack)? else {
                        return Ok(None);
                    };
                    values.push(value);
                }
                Ok(Some(Value::List(values)))
            }
            ValueAst::Map(entries) => {
                let mut map = BTreeMap::new();
                for (key, value) in entries {
                    let Some(value) = self.expr_value(value, locals, site, stack)? else {
                        return Ok(None);
                    };
                    map.insert(key.clone(), value);
                }
                Ok(Some(Value::Map(map)))
            }
            ValueAst::Reference(reference) => {
                let path = Path::from_segments(reference.clone())
                    .map_err(|error| Error::eval(site.to_string(), error.to_string()))?;
                self.resolve_path(&path, locals, stack)
            }
        }
    }

    fn resolve_path(
        &self,
        path: &Path,
        locals: &Locals,
        stack: &mut Vec<Path>,
    ) -> Result<Option<Value>, Error> {
        if let Some(value) = resolve_local(locals, path)? {
            return Ok(Some(value));
        }

        if stack.contains(path) {
            return Err(Error::eval(path.to_string(), "reference cycle"));
        }

        stack.push(path.clone());
        let mut matches = Vec::new();
        let mut match_error = None;
        for decl in self.decls {
            if decl.path == *path {
                match self.active(decl, stack) {
                    Ok(true) => matches.push(decl),
                    Ok(false) => {}
                    Err(error) => {
                        match_error = Some(error);
                        break;
                    }
                }
            }
        }
        let value = match match_error {
            Some(error) => {
                stack.pop();
                return Err(error);
            }
            None => {
                if matches.len() > 1 {
                    stack.pop();
                    return Err(Error::eval(
                        path.to_string(),
                        "reference is ambiguous: more than one active declaration",
                    ));
                }
                match matches.first() {
                    Some(decl) => self.decl_value(decl, stack)?,
                    None => None,
                }
            }
        };
        stack.pop();
        if value.is_some() {
            return Ok(value);
        }

        if let Some(value) = self.assemble(path, locals, stack)? {
            return Ok(Some(value));
        }

        Ok(self.context_value(path))
    }

    fn decl_value(&self, decl: &Decl, stack: &mut Vec<Path>) -> Result<Option<Value>, Error> {
        match &decl.value {
            DeclValue::Expr(ast) => self.expr_value(ast, &decl.locals, &decl.path, stack),
            DeclValue::Block(body) => {
                let map = self.block(body, &decl.path, &decl.locals, stack)?;
                Ok(Some(Value::Map(map)))
            }
        }
    }

    fn assemble(
        &self,
        path: &Path,
        locals: &Locals,
        stack: &mut Vec<Path>,
    ) -> Result<Option<Value>, Error> {
        if stack.contains(path) {
            return Err(Error::eval(path.to_string(), "reference cycle"));
        }
        let Ok(node) = self.root.lookup(path) else {
            return Ok(None);
        };
        stack.push(path.clone());
        let result = self.assemble_node(node, path, locals, stack);
        stack.pop();
        result
    }

    fn assemble_node(
        &self,
        node: &Node,
        path: &Path,
        locals: &Locals,
        stack: &mut Vec<Path>,
    ) -> Result<Option<Value>, Error> {
        let mut children = BTreeSet::new();
        for decl in self.decls {
            if let Some(rest) = decl.path.strip_prefix(path) {
                if let Some(first) = rest.segments().first() {
                    if self.active(decl, stack)? {
                        children.insert(first.clone());
                    }
                }
            }
        }
        if children.is_empty() {
            return Ok(None);
        }

        match node.kind {
            NodeKind::Struct => {
                let mut map = BTreeMap::new();
                for child in children {
                    let child_path = path
                        .child(&child)
                        .map_err(|error| Error::eval(path.to_string(), error.to_string()))?;
                    let Some(value) = self.resolve_path(&child_path, locals, stack)? else {
                        return Err(Error::eval(
                            child_path.to_string(),
                            "object reference has an unresolved field",
                        ));
                    };
                    map.insert(child, value);
                }
                Ok(Some(Value::Map(map)))
            }
            NodeKind::Map => {
                let mut map = BTreeMap::new();
                for child in children {
                    let child_path = path
                        .child(&child)
                        .map_err(|error| Error::eval(path.to_string(), error.to_string()))?;
                    let Some(value) = self.resolve_path(&child_path, locals, stack)? else {
                        return Err(Error::eval(
                            child_path.to_string(),
                            "object reference has an unresolved field",
                        ));
                    };
                    map.insert(child, value);
                }
                Ok(Some(Value::Map(map)))
            }
            NodeKind::List if node.key_field.is_some() => {
                let key_field = node.key_field.as_deref().expect("keyed list");
                let mut items = Vec::new();
                for key in children {
                    let child_path = path
                        .child(&key)
                        .map_err(|error| Error::eval(path.to_string(), error.to_string()))?;
                    let Some(value) = self.resolve_path(&child_path, locals, stack)? else {
                        return Err(Error::eval(
                            child_path.to_string(),
                            "object reference has an unresolved element",
                        ));
                    };
                    let mut map = match value {
                        Value::Map(map) => map,
                        other => {
                            return Err(Error::eval(
                                child_path.to_string(),
                                format!(
                                    "a keyed list element is an object, found {}",
                                    other.type_name()
                                ),
                            ));
                        }
                    };
                    map.insert(key_field.to_string(), Value::String(key));
                    items.push(Value::Map(map));
                }
                Ok(Some(Value::List(items)))
            }
            _ => Ok(None),
        }
    }

    fn context_value(&self, path: &Path) -> Option<Value> {
        if let Some(value) = self.context.get(path) {
            return Some(value.clone());
        }
        for (key, value) in &self.context.values {
            if let Some(rest) = path.strip_prefix(key) {
                if let Ok(value) = traverse_value(value, rest.segments()) {
                    return value;
                }
            }
        }
        let mut map = BTreeMap::new();
        let mut any = false;
        for (key, value) in &self.context.values {
            if let Some(rest) = key.strip_prefix(path) {
                if rest.is_root() {
                    return Some(value.clone());
                }
                any = true;
                let _ = insert_nested(&mut map, rest.segments(), value.clone(), path);
            }
        }
        any.then_some(Value::Map(map))
    }
}

fn combine_or(parts: Vec<Expr>, site: &Path) -> Result<Expr, Error> {
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
    if parts.iter().any(|part| !part.constraints.is_empty()) {
        let mut output = Expr::default();
        for part in parts {
            output = output.merge(part);
        }
        return Ok(output);
    }
    Err(Error::eval(
        site.to_string(),
        "disjunction mixes values, constraints and natives",
    ))
}

fn effective(expr: Expr, site: &Path) -> Result<Option<Value>, Error> {
    let candidates: BTreeSet<Value> = expr.candidates.into_iter().collect();
    if !candidates.is_empty() {
        if candidates.len() == 1 {
            return Ok(candidates.into_iter().next());
        }
        return Err(Error::eval(
            site.to_string(),
            "expression has more than one concrete candidate",
        ));
    }
    let defaults: BTreeSet<Value> = expr.defaults.into_iter().collect();
    if !defaults.is_empty() {
        if defaults.len() == 1 {
            return Ok(defaults.into_iter().next());
        }
        return Err(Error::eval(
            site.to_string(),
            "expression has more than one concrete default",
        ));
    }
    Ok(None)
}

fn resolve_local(locals: &Locals, path: &Path) -> Result<Option<Value>, Error> {
    let Some(first) = path.segments().first() else {
        return Ok(None);
    };
    let Some(mut value) = locals.get(first).cloned() else {
        return Ok(None);
    };
    for segment in &path.segments()[1..] {
        value = match value {
            Value::Map(mut map) => match map.remove(segment) {
                Some(value) => value,
                None => {
                    return Err(Error::eval(
                        path.to_string(),
                        format!("binding '{first}' has no field '{segment}'"),
                    ));
                }
            },
            Value::List(items) => {
                let index: usize = segment.parse().map_err(|_| {
                    Error::eval(
                        path.to_string(),
                        format!("binding '{first}' is a list and '{segment}' is not an index"),
                    )
                })?;
                items.get(index).cloned().ok_or_else(|| {
                    Error::eval(
                        path.to_string(),
                        format!("binding '{first}' has no index {index}"),
                    )
                })?
            }
            other => {
                return Err(Error::eval(
                    path.to_string(),
                    format!("binding '{first}' is {}, not an object", other.type_name()),
                ));
            }
        };
    }
    Ok(Some(value))
}

fn traverse_value(value: &Value, segments: &[String]) -> Result<Option<Value>, ()> {
    let mut value = value.clone();
    for segment in segments {
        value = match value {
            Value::Map(mut map) => match map.remove(segment) {
                Some(value) => value,
                None => return Ok(None),
            },
            Value::List(items) => {
                let Ok(index) = segment.parse::<usize>() else {
                    return Ok(None);
                };
                match items.get(index) {
                    Some(value) => value.clone(),
                    None => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
    }
    Ok(Some(value))
}

fn scalar_text(value: &Value, site: &Path) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Int(value) => Ok(value.to_string()),
        Value::Float(value) => Ok(value.get().to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => Ok("null".to_string()),
        Value::List(_) | Value::Map(_) | Value::Native { .. } => Err(Error::eval(
            site.to_string(),
            format!("interpolation needs a scalar, found {}", value.type_name()),
        )),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0,
        Value::Float(value) => value.get() != 0.0,
        Value::String(value) => !value.is_empty(),
        Value::List(values) => !values.is_empty(),
        Value::Map(values) => !values.is_empty(),
        Value::Native { .. } => true,
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    if matches!(left, Value::Int(_) | Value::Float(_))
        && matches!(right, Value::Int(_) | Value::Float(_))
    {
        return Value::compare_scalar(left, right) == Some(std::cmp::Ordering::Equal);
    }
    left == right
}

fn insert_nested(
    map: &mut BTreeMap<String, Value>,
    segments: &[String],
    value: Value,
    site: &Path,
) -> Result<(), Error> {
    let Some((first, rest)) = segments.split_first() else {
        return Err(Error::eval(site.to_string(), "a value needs a key"));
    };
    if rest.is_empty() {
        map.insert(first.clone(), value);
        return Ok(());
    }
    let entry = map
        .entry(first.clone())
        .or_insert_with(|| Value::Map(BTreeMap::new()));
    match entry {
        Value::Map(inner) => insert_nested(inner, rest, value, site),
        _ => Err(Error::eval(
            site.to_string(),
            format!("'{first}' is both a value and an object"),
        )),
    }
}

fn merge_maps(
    target: &mut BTreeMap<String, Value>,
    source: BTreeMap<String, Value>,
    site: &Path,
) -> Result<(), Error> {
    for (key, value) in source {
        match (target.get_mut(&key), value) {
            (Some(Value::Map(left)), Value::Map(right)) => merge_maps(left, right, site)?,
            (Some(_), _) => {
                return Err(Error::eval(
                    site.to_string(),
                    format!("'{key}' is declared more than once in a block"),
                ));
            }
            (None, value) => {
                target.insert(key, value);
            }
        }
    }
    Ok(())
}
