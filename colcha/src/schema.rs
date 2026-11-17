use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;

use crate::path::Path;
use crate::value::{Float, Value};

/// Identity of the root Rust type that produced a fact.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SchemaId {
    pub name: String,
    pub version: u32,
}

impl SchemaId {
    pub fn new(name: impl Into<String>, version: u32) -> SchemaId {
        SchemaId {
            name: name.into(),
            version,
        }
    }
}

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// Derive-time rule that decides the winner at one leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum FoldPolicy {
    Lww,
    Flat,
    Union,
    Atomic,
    Native,
}

/// Shape of a node in the derived schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum NodeKind {
    Leaf,
    Raw,
    Struct,
    List,
    Map,
    Native,
}

/// Schema node derived from a Rust type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Node {
    pub kind: NodeKind,
    pub policy: FoldPolicy,
    pub constraints: Vec<Constraint>,
    pub default: Option<Value>,
    pub fields: BTreeMap<String, Node>,
    pub element: Option<Box<Node>>,
    pub key_field: Option<String>,
    pub natives: BTreeSet<String>,
    pub optional: bool,
}

impl Default for Node {
    fn default() -> Node {
        Node {
            kind: NodeKind::Leaf,
            policy: FoldPolicy::Lww,
            constraints: Vec::new(),
            default: None,
            fields: BTreeMap::new(),
            element: None,
            key_field: None,
            natives: BTreeSet::new(),
            optional: false,
        }
    }
}

impl Node {
    pub fn leaf(policy: FoldPolicy) -> Node {
        Node {
            kind: NodeKind::Leaf,
            policy,
            ..Node::default()
        }
    }

    pub fn structure() -> Node {
        Node {
            kind: NodeKind::Struct,
            ..Node::default()
        }
    }

    /// A leaf that accepts any concrete value.
    pub fn raw() -> Node {
        Node {
            kind: NodeKind::Raw,
            ..Node::default()
        }
    }

    pub fn list(element: Node, key_field: Option<String>) -> Node {
        Node {
            kind: NodeKind::List,
            policy: if key_field.is_some() {
                FoldPolicy::Lww
            } else {
                FoldPolicy::Atomic
            },
            element: Some(Box::new(element)),
            key_field,
            ..Node::default()
        }
    }

    pub fn map(element: Node) -> Node {
        Node {
            kind: NodeKind::Map,
            element: Some(Box::new(element)),
            ..Node::default()
        }
    }

    pub fn native(name: impl Into<String>) -> Node {
        let name = name.into();
        let mut natives = BTreeSet::new();
        natives.insert(name);
        Node {
            kind: NodeKind::Native,
            policy: FoldPolicy::Native,
            natives,
            ..Node::default()
        }
    }

    pub fn with_constraint(mut self, constraint: Constraint) -> Node {
        self.constraints.push(constraint);
        self
    }

    pub fn with_default(mut self, value: Value) -> Node {
        self.default = Some(value);
        self
    }

    pub fn optional(mut self) -> Node {
        self.optional = true;
        self
    }

    /// Resolve a path to its schema node and reject writes to a derived key.
    pub fn lookup(&self, path: &Path) -> Result<&Node, String> {
        let mut node = self;
        let segments = path.segments();
        let mut index = 0;
        while index < segments.len() {
            let segment = &segments[index];
            match node.kind {
                NodeKind::Struct => {
                    if node.key_field.as_deref() == Some(segment.as_str()) {
                        return Err(format!("field '{segment}' is derived from the path key"));
                    }
                    node = node
                        .fields
                        .get(segment)
                        .ok_or_else(|| format!("unknown field '{segment}'"))?;
                    index += 1;
                }
                NodeKind::List => {
                    if node.key_field.is_some() {
                        node = node
                            .element
                            .as_deref()
                            .ok_or_else(|| "list has no element schema".to_string())?;
                        index += 1;
                    } else {
                        return Err("an atomic list has no child paths".to_string());
                    }
                }
                NodeKind::Map => {
                    node = node
                        .element
                        .as_deref()
                        .ok_or_else(|| "map has no element schema".to_string())?;
                    index += 1;
                }
                NodeKind::Leaf => {
                    return Err(format!("'{segment}' is below a scalar leaf"));
                }
                NodeKind::Raw => {
                    return Err(format!("'{segment}' is below a raw leaf"));
                }
                NodeKind::Native => {
                    // Native argument paths are validated when the registry is known.
                    return Ok(node);
                }
            }
        }
        Ok(node)
    }
}

/// Pure predicate that narrows the accepted values of a leaf.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Constraint {
    Type(String),
    Min(Value),
    Max(Value),
    Greater(Value),
    Less(Value),
    Regex(String),
    OneOf(Vec<Value>),
    NativeOneOf(Vec<String>),
}

impl Constraint {
    pub fn type_name(name: impl Into<String>) -> Constraint {
        Constraint::Type(name.into())
    }

    pub fn satisfies(&self, value: &Value) -> Result<bool, String> {
        match self {
            Constraint::Type(name) => Ok(type_accepts(name, value)),
            Constraint::Min(bound) => Ok(Value::compare_scalar(value, bound)
                .map(|ordering| ordering != std::cmp::Ordering::Less)
                .unwrap_or(false)),
            Constraint::Max(bound) => Ok(Value::compare_scalar(value, bound)
                .map(|ordering| ordering != std::cmp::Ordering::Greater)
                .unwrap_or(false)),
            Constraint::Greater(bound) => Ok(Value::compare_scalar(value, bound)
                .map(|ordering| ordering == std::cmp::Ordering::Greater)
                .unwrap_or(false)),
            Constraint::Less(bound) => Ok(Value::compare_scalar(value, bound)
                .map(|ordering| ordering == std::cmp::Ordering::Less)
                .unwrap_or(false)),
            Constraint::Regex(pattern) => {
                let regex = regex::Regex::new(pattern)
                    .map_err(|error| format!("invalid regex '{pattern}': {error}"))?;
                Ok(match value {
                    Value::String(text) => regex.is_match(text),
                    _ => false,
                })
            }
            Constraint::OneOf(values) => Ok(values.iter().any(|candidate| candidate == value)),
            Constraint::NativeOneOf(names) => Ok(match value {
                Value::Native { name, .. } => names.iter().any(|allowed| allowed == name),
                Value::String(name) => names.iter().any(|allowed| allowed == name),
                _ => false,
            }),
        }
    }

    /// Returns a reason when the two constraints cannot be satisfied together.
    pub fn conflict_with(&self, other: &Constraint) -> Option<String> {
        match (self, other) {
            (Constraint::Type(left), Constraint::Type(right)) => {
                if types_compatible(left, right) {
                    None
                } else {
                    Some(format!("types '{left}' and '{right}' cannot meet"))
                }
            }
            (Constraint::Type(name), Constraint::OneOf(values))
            | (Constraint::OneOf(values), Constraint::Type(name)) => {
                if values.iter().any(|value| type_accepts(name, value)) {
                    None
                } else {
                    Some(format!("no value of type '{name}' is in the allowed set"))
                }
            }
            (
                Constraint::Min(_)
                | Constraint::Max(_)
                | Constraint::Greater(_)
                | Constraint::Less(_),
                Constraint::Min(_)
                | Constraint::Max(_)
                | Constraint::Greater(_)
                | Constraint::Less(_),
            ) => bound_conflict(self, other),
            (
                Constraint::OneOf(values),
                Constraint::Min(_)
                | Constraint::Max(_)
                | Constraint::Greater(_)
                | Constraint::Less(_),
            )
            | (
                Constraint::Min(_)
                | Constraint::Max(_)
                | Constraint::Greater(_)
                | Constraint::Less(_),
                Constraint::OneOf(values),
            ) => {
                if values.iter().any(|value| {
                    self.satisfies(value).unwrap_or(false)
                        && other.satisfies(value).unwrap_or(false)
                }) {
                    None
                } else {
                    Some("no allowed value satisfies the numeric bound".to_string())
                }
            }
            (Constraint::OneOf(left), Constraint::OneOf(right)) => {
                if left.iter().any(|value| right.contains(value)) {
                    None
                } else {
                    Some("the allowed sets are disjoint".to_string())
                }
            }
            (Constraint::NativeOneOf(left), Constraint::NativeOneOf(right)) => {
                if left.iter().any(|name| right.contains(name)) {
                    None
                } else {
                    Some("the allowed native sets are disjoint".to_string())
                }
            }
            (Constraint::Type(name), Constraint::Regex(_))
            | (Constraint::Regex(_), Constraint::Type(name)) => {
                if name == "string" || name == "float" || name == "int" {
                    None
                } else {
                    Some(format!("a regex cannot narrow type '{name}'"))
                }
            }
            (
                Constraint::Type(name),
                Constraint::Min(value)
                | Constraint::Max(value)
                | Constraint::Greater(value)
                | Constraint::Less(value),
            ) => {
                if matches!(value, Value::Int(_) | Value::Float(_))
                    && !matches!(name.as_str(), "int" | "float")
                {
                    Some(format!("a numeric bound cannot narrow type '{name}'"))
                } else {
                    None
                }
            }
            (
                Constraint::Min(value)
                | Constraint::Max(value)
                | Constraint::Greater(value)
                | Constraint::Less(value),
                Constraint::Type(name),
            ) => {
                if matches!(value, Value::Int(_) | Value::Float(_))
                    && !matches!(name.as_str(), "int" | "float")
                {
                    Some(format!("a numeric bound cannot narrow type '{name}'"))
                } else {
                    None
                }
            }
            (Constraint::Regex(pattern), Constraint::OneOf(values))
            | (Constraint::OneOf(values), Constraint::Regex(pattern)) => {
                let Ok(regex) = regex::Regex::new(pattern) else {
                    return Some(format!("invalid regex '{pattern}'"));
                };
                if values
                    .iter()
                    .any(|value| matches!(value, Value::String(text) if regex.is_match(text)))
                {
                    None
                } else {
                    Some("no allowed value matches the regex".to_string())
                }
            }
            _ => None,
        }
    }
}

fn lower_bound(constraint: &Constraint) -> Option<(&Value, bool)> {
    match constraint {
        Constraint::Min(value) => Some((value, true)),
        Constraint::Greater(value) => Some((value, false)),
        _ => None,
    }
}

fn upper_bound(constraint: &Constraint) -> Option<(&Value, bool)> {
    match constraint {
        Constraint::Max(value) => Some((value, true)),
        Constraint::Less(value) => Some((value, false)),
        _ => None,
    }
}

fn bound_conflict(left: &Constraint, right: &Constraint) -> Option<String> {
    for (lower, upper) in [(left, right), (right, left)] {
        let (Some((low, low_inclusive)), Some((high, high_inclusive))) =
            (lower_bound(lower), upper_bound(upper))
        else {
            continue;
        };
        match Value::compare_scalar(low, high) {
            Some(std::cmp::Ordering::Greater) => {
                return Some(format!("lower bound {low:?} is above upper bound {high:?}"));
            }
            Some(std::cmp::Ordering::Equal) if !low_inclusive || !high_inclusive => {
                return Some("the bounds meet without an allowed value".to_string());
            }
            _ => {}
        }
    }
    None
}

fn type_accepts(name: &str, value: &Value) -> bool {
    match name {
        "float" => matches!(value, Value::Float(_) | Value::Int(_)),
        "int" => matches!(value, Value::Int(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" => matches!(value, Value::Bool(_)),
        "null" => matches!(value, Value::Null),
        "list" => matches!(value, Value::List(_)),
        "map" => matches!(value, Value::Map(_)),
        _ => false,
    }
}

fn types_compatible(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    matches!((left, right), ("int", "float") | ("float", "int"))
}

/// Host-provided native type description.
#[derive(Clone)]
pub struct Native {
    pub name: String,
    pub args: Node,
    pub coerce: Option<fn(&Value) -> Option<Value>>,
}

impl Native {
    pub fn new(name: impl Into<String>, args: Node) -> Native {
        Native {
            name: name.into(),
            args,
            coerce: None,
        }
    }

    pub fn with_coerce(mut self, coerce: fn(&Value) -> Option<Value>) -> Native {
        self.coerce = Some(coerce);
        self
    }
}

impl fmt::Debug for Native {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Native")
            .field("name", &self.name)
            .field("args", &self.args)
            .field("coerce", &self.coerce.map(|_| "<fn>"))
            .finish()
    }
}

/// Implemented by the root Rust type and by derived element types.
pub trait Schema {
    const NAME: &'static str;
    const VERSION: u32 = 1;
    fn schema() -> Node;
}

/// Convenience constructor used by tests and by generated code.
pub fn scalar(value: f64) -> Option<Value> {
    Float::new(value).map(Value::Float)
}
